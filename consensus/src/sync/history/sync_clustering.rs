use std::collections::VecDeque;
use std::sync::{Arc, Weak};

use parking_lot::RwLock;

use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_network_interface::prelude::{CloseReason, Network, Peer};

use crate::consensus_agent::ConsensusAgent;
use crate::messages::{BlockHashType, RequestBlockHashesFilter};
use crate::sync::history::cluster::{SyncCluster, SyncClusterResult};
use crate::sync::history::sync::EpochIds;
use crate::sync::history::HistorySync;
use crate::sync::request_component::HistorySyncStream;

impl<TNetwork: Network> HistorySync<TNetwork> {
    pub(crate) async fn request_epoch_ids(
        blockchain: Arc<RwLock<Blockchain>>,
        agent: Arc<ConsensusAgent<TNetwork::PeerType>>,
    ) -> Option<EpochIds<TNetwork::PeerType>> {
        let (locators, epoch_number) = {
            // Order matters here. The first hash found by the recipient of the request will be
            // used, so they need to be in backwards block height order.
            let blockchain = blockchain.read();
            let election_head = blockchain.election_head();
            let macro_head = blockchain.macro_head();

            // So if there is a checkpoint hash that should be included in addition to the election
            // block hash, it should come first.
            let mut locators = vec![];
            if macro_head.hash() != election_head.hash() {
                locators.push(macro_head.hash());
            }
            // The election bock is at the end here
            locators.push(election_head.hash());

            (locators, election_head.epoch_number())
        };

        let result = agent
            .request_block_hashes(
                locators,
                1000, // TODO: Use other value
                RequestBlockHashesFilter::ElectionAndLatestCheckpoint,
            )
            .await;

        match result {
            Ok(block_hashes) => {
                if block_hashes.hashes.is_none() {
                    return Some(EpochIds {
                        locator_found: false,
                        ids: Vec::new(),
                        checkpoint_id: None,
                        first_epoch_number: 0,
                        sender: agent,
                    });
                }

                let hashes = block_hashes.hashes.unwrap();

                // Get checkpoint id if exists.
                let checkpoint_id = hashes.last().and_then(|(ty, id)| match *ty {
                    BlockHashType::Checkpoint => Some(id.clone()),
                    _ => None,
                });

                // Filter checkpoint from block hashes and map to hash.
                let epoch_ids = hashes
                    .into_iter()
                    .filter_map(|(ty, id)| match ty {
                        BlockHashType::Election => Some(id),
                        _ => None,
                    })
                    .collect();

                Some(EpochIds {
                    locator_found: true,
                    ids: epoch_ids,
                    checkpoint_id,
                    first_epoch_number: epoch_number as usize + 1,
                    sender: agent,
                })
            }
            Err(e) => {
                log::error!("Request block hashes failed: {}", e);
                agent.peer.close(CloseReason::Other);
                None
            }
        }
    }

    pub(crate) fn cluster_epoch_ids(&mut self, mut epoch_ids: EpochIds<TNetwork::PeerType>) {
        let checkpoint_epoch = epoch_ids.get_checkpoint_epoch();
        let agent = epoch_ids.sender;

        let (current_id, election_head) = {
            let blockchain = self.blockchain.read();
            (blockchain.election_head_hash(), blockchain.election_head())
        };

        // If `epoch_ids` includes known blocks, truncate (or discard on fork prior to our accepted state).
        let current_epoch = election_head.epoch_number() as usize;
        if !epoch_ids.ids.is_empty() && epoch_ids.first_epoch_number <= current_epoch {
            // Check most recent id against our state.
            // FIXME Also check upper epoch number to see if the last epoch id actually corresponds
            //  to our state.
            if current_id == epoch_ids.ids[current_epoch - epoch_ids.first_epoch_number] {
                // Remove known blocks.
                epoch_ids.ids = epoch_ids
                    .ids
                    .split_off(current_epoch - epoch_ids.first_epoch_number + 1);
                epoch_ids.first_epoch_number = current_epoch;

                // If there are no new election blocks left, return.
                if epoch_ids.ids.is_empty() {
                    return;
                }
            } else {
                // TODO: Improve debug output.
                debug!("Got fork prior to our accepted state.");
                return;
            }
        }

        let mut id_index = 0;
        let mut new_clusters = VecDeque::new();
        let mut num_clusters = 0;

        trace!(
            "Clustering ids: first_epoch_number={}, num_ids={}, num_clusters={}, active_cluster={}",
            epoch_ids.first_epoch_number,
            epoch_ids.ids.len(),
            self.epoch_clusters.len(),
            self.active_cluster.is_some(),
        );

        let epoch_clusters = self
            .epoch_clusters
            .iter_mut()
            .chain(self.active_cluster.iter_mut());
        for cluster in epoch_clusters {
            // Check if given epoch_ids and the current cluster potentially overlap.
            if cluster.first_epoch_number <= epoch_ids.first_epoch_number
                && cluster.first_epoch_number + cluster.ids.len() > epoch_ids.first_epoch_number
            {
                // Compare epoch ids in the overlapping region.
                let start_offset = epoch_ids.first_epoch_number - cluster.first_epoch_number;
                let len = usize::min(
                    cluster.ids.len() - start_offset,
                    epoch_ids.ids.len() - id_index,
                );
                let match_until = cluster.ids[start_offset..start_offset + len]
                    .iter()
                    .zip(&epoch_ids.ids[id_index..id_index + len])
                    .position(|(first, second)| first != second)
                    .unwrap_or(len);

                trace!(
                    "Comparing with cluster: first_epoch_number={}, num_ids={}, match_until={}",
                    cluster.first_epoch_number,
                    cluster.ids.len(),
                    match_until
                );

                // If there is no match at all, skip to the next cluster.
                if match_until > 0 {
                    // If there is only a partial match, split the current cluster. The current cluster
                    // is truncated to the matching overlapping part and the removed ids are put in a new
                    // cluster. Buffer up the new clusters and insert them after we finish iterating over
                    // sync_clusters.
                    if match_until < cluster.ids.len() - start_offset {
                        trace!(
                            "Splitting cluster: num_ids={}, start_offset={}, split_at={}",
                            cluster.ids.len(),
                            start_offset,
                            start_offset + match_until
                        );
                        new_clusters.push_back(cluster.split_off(start_offset + match_until));
                    }

                    // The peer's epoch ids matched at least a part of this (now potentially truncated) cluster,
                    // so we add the peer to this cluster. We also increment the peer's number of clusters.
                    cluster.add_peer(Arc::downgrade(&agent));
                    num_clusters += 1;

                    // Advance the id_index by the number of matched ids.
                    // If there are no more ids to cluster, we can stop iterating.
                    id_index += match_until;
                    if id_index >= epoch_ids.ids.len() {
                        break;
                    }
                }
            }
        }

        // Add remaining ids to a new cluster with only the sending peer in it.
        if id_index < epoch_ids.ids.len() {
            trace!(
                "Adding new cluster: id_index={}, first_epoch_number={}, num_ids={}",
                id_index,
                epoch_ids.first_epoch_number + id_index,
                epoch_ids.ids.len() - id_index
            );
            new_clusters.push_back(SyncCluster::new(
                Vec::from(&epoch_ids.ids[id_index..]),
                epoch_ids.first_epoch_number + id_index,
                vec![Arc::downgrade(&agent)],
                Arc::clone(&self.blockchain),
            ));
            // Don't increment the num_clusters here, as this is done in the loop later on.
        }

        // Now cluster the checkpoint id if present.
        if let Some(checkpoint_id) = epoch_ids.checkpoint_id {
            let mut found_cluster = false;
            let checkpoint_clusters = self
                .checkpoint_clusters
                .iter_mut()
                .chain(self.active_cluster.iter_mut());
            for cluster in checkpoint_clusters {
                // Currently, we do not need to remove old checkpoint ids from the same peer.
                // Since we only request new epoch ids (and checkpoints) once a peer has 0 clusters,
                // we can never receive an updated checkpoint.
                // When this invariant changes, we need to remove old checkpoints of that peer here!

                // Look for clusters at the same epoch with the same hash.
                if cluster.first_epoch_number == checkpoint_epoch
                    && cluster.ids[0] == checkpoint_id
                    && cluster.ids.len() == 1
                {
                    // The peer's checkpoint id matched this cluster,
                    // so we add the peer to this cluster. We also increment the peer's number of clusters.
                    cluster.add_peer(Arc::downgrade(&agent));
                    num_clusters += 1;
                    found_cluster = true;
                    break;
                }
            }

            // If there was no suitable cluster, add a new one.
            if !found_cluster {
                let cluster = SyncCluster::new(
                    vec![checkpoint_id],
                    checkpoint_epoch,
                    vec![Arc::downgrade(&agent)],
                    Arc::clone(&self.blockchain),
                );
                self.checkpoint_clusters.push_back(cluster);
                num_clusters += 1;
            }
        }

        // Store agent Arc and number of clusters it's in.
        self.agents
            .insert(Arc::clone(&agent.peer), (agent, num_clusters));

        // Update cluster counts for all peers in new clusters.
        for cluster in &new_clusters {
            for agent in cluster.peers() {
                if let Some(agent) = Weak::upgrade(agent) {
                    let pair = self
                        .agents
                        .get_mut(&agent.peer)
                        .expect("Agent should be present");
                    pair.1 += 1;
                }
            }
        }

        // Add buffered clusters to sync_clusters.
        self.epoch_clusters.append(&mut new_clusters);
    }

    pub(crate) fn pop_next_cluster(&mut self) -> Option<SyncCluster<TNetwork::PeerType>> {
        let cluster =
            HistorySync::<TNetwork>::find_best_cluster(&mut self.epoch_clusters, &self.blockchain);

        // If we made space in epoch_clusters, wake the task.
        if cluster.is_some() {
            if let Some(waker) = self.waker.take() {
                waker.wake();
            }
            return cluster;
        }

        HistorySync::<TNetwork>::find_best_cluster(&mut self.checkpoint_clusters, &self.blockchain)
    }

    fn find_best_cluster(
        clusters: &mut VecDeque<SyncCluster<TNetwork::PeerType>>,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) -> Option<SyncCluster<TNetwork::PeerType>> {
        if clusters.is_empty() {
            return None;
        }

        let current_epoch = blockchain.read().election_head().epoch_number() as usize;

        let (best_idx, _) = clusters
            .iter()
            .enumerate()
            .reduce(|a, b| {
                if a.1.compare(b.1, current_epoch).is_gt() {
                    a
                } else {
                    b
                }
            })
            .expect("clusters not empty");

        let mut best_cluster = clusters
            .swap_remove_front(best_idx)
            .expect("best cluster should be there");

        debug!("Syncing cluster at index {} out of {} clusters: current_epoch={}, first_epoch_number={}, num_ids={}, num_peers: {}",
               best_idx, clusters.len() + 1, current_epoch, best_cluster.first_epoch_number, best_cluster.ids.len(), best_cluster.peers().len());

        if best_cluster.first_epoch_number <= current_epoch {
            best_cluster.remove_front(current_epoch - best_cluster.first_epoch_number + 1);
        }

        Some(best_cluster)
    }

    /// Reduces the number of clusters for each peer present in the given cluster by 1.
    ///
    /// If for any given peer the cluster count falls to zero and the cluster didn't error,
    /// a request for more epoch ids will be send to the peer.
    ///
    /// Peers with no clusters are always removed from the agent set as they are re added if they
    /// provide new epoch ids or emitted as synced peers if there are no new ids to sync.
    pub(crate) fn finish_cluster(
        &mut self,
        cluster: &SyncCluster<TNetwork::PeerType>,
        result: SyncClusterResult,
    ) {
        if result != SyncClusterResult::NoMoreEpochs {
            debug!("Failed to push epoch: {:?}", result);
        }

        // Decrement the cluster count for all peers in the cluster.
        for peer in cluster.peers() {
            if let Some(agent) = Weak::upgrade(peer) {
                let cluster_count = {
                    let pair = self
                        .agents
                        .get_mut(&agent.peer)
                        .expect("Agent should be present");
                    pair.1 -= 1;
                    pair.1
                };

                // If the peer isn't in any more clusters, request more epoch_ids from it.
                // Only do so if the cluster was synced.
                if cluster_count == 0 {
                    // Always remove agent from agents map. It will be re-added if it returns more
                    // epoch_ids and dropped otherwise.
                    self.agents.remove(&agent.peer);

                    if result != SyncClusterResult::Error {
                        self.add_agent(agent);
                    } else {
                        agent.peer.close(CloseReason::Other);
                    }
                }
            }
        }
    }
}
