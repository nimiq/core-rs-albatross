use std::collections::VecDeque;
use std::sync::{Arc, Weak};

use parking_lot::RwLock;

use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::prelude::{CloseReason, Network, Peer};

use crate::consensus_agent::ConsensusAgent;
use crate::messages::{BlockHashType, RequestBlockHashesFilter};
use crate::sync::history::cluster::{SyncCluster, SyncClusterResult};
use crate::sync::history::sync::{EpochIds, Job};
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
                let epoch_ids: Vec<Blake2bHash> = hashes
                    .into_iter()
                    .filter_map(|(ty, id)| match ty {
                        BlockHashType::Election => Some(id),
                        _ => None,
                    })
                    .collect();

                log::debug!(
                    "Received {} epoch_ids starting at #{} (checkpoint={}) from {:?}",
                    epoch_ids.len(),
                    epoch_number + 1,
                    checkpoint_id.is_some(),
                    agent.peer.id(),
                );

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

    pub(crate) fn cluster_epoch_ids(
        &mut self,
        mut epoch_ids: EpochIds<TNetwork::PeerType>,
    ) -> Option<Arc<ConsensusAgent<TNetwork::PeerType>>> {
        // Read our current blockchain state.
        let (our_epoch_id, our_epoch_number) = {
            let blockchain = self.blockchain.read();
            (
                blockchain.election_head_hash(),
                blockchain.election_head().epoch_number() as usize,
            )
        };

        // Truncate epoch_ids by epoch_number: Discard all epoch_ids prior to our accepted state.
        if !epoch_ids.ids.is_empty() && epoch_ids.first_epoch_number <= our_epoch_number {
            // Check that the epoch_id sent by the peer at our current epoch number corresponds to
            // out accepted state. If it doesn't, the peer is on a "permanent" fork, so we ban it.
            let peers_epoch_id = &epoch_ids.ids[our_epoch_number - epoch_ids.first_epoch_number];
            if our_epoch_id != *peers_epoch_id {
                // TODO Actually ban the peer.
                return Some(epoch_ids.sender);
            }

            epoch_ids.ids = epoch_ids
                .ids
                .split_off(our_epoch_number - epoch_ids.first_epoch_number + 1);
            epoch_ids.first_epoch_number = our_epoch_number + 1;
        }

        // TODO Sanity check: All of the remaining ids should be unknown

        // Check if we have already downloaded the remaining epoch_ids but not applied them to the
        // blockchain yet. Iterate over epoch_ids and job_queue in parallel, as we expect epochs
        // to appear in the same order.
        // TODO Currently, we don't remove known ids if they appear in a different order than in the
        //  job queue. If we validated the macro block signature of each epoch as soon as we get the
        //  macro block for an epoch (before downloading the history), we would avoid downloading
        //  invalid epochs and could reject out-of-order ids here immediately.
        let mut id_iter = epoch_ids.ids.iter().chain(epoch_ids.checkpoint_id.iter());
        let mut job_iter = self.job_queue.iter_mut();

        let mut num_ids_to_remove = 0;
        let mut cluster_id = 0;
        'outer: while let Some(id) = id_iter.next() {
            loop {
                let job = match job_iter.next() {
                    Some(job) => job,
                    None => break 'outer,
                };

                if let Job::PushBatchSet(cid, batch_set_id, _) = job {
                    if id == batch_set_id {
                        num_ids_to_remove += 1;
                        cluster_id = *cid;
                        break;
                    }
                }
            }
        }

        // Check if we removed all ids (including the checkpoint id if it existed).
        if num_ids_to_remove > epoch_ids.ids.len()
            || (num_ids_to_remove == epoch_ids.ids.len() && epoch_ids.checkpoint_id.is_none())
        {
            // No ids remain, nothing new to learn from this peer at this point.
            //
            let cluster = job_iter.find_map(|job| match job {
                Job::FinishCluster(cluster, _) if cluster.id == cluster_id => Some(cluster),
                _ => None,
            });

            // If a FinishCluster job exists, store the peer in the finished cluster so we request
            // more epoch ids from it when the job is processed.
            if let Some(cluster) = cluster {
                let agent = epoch_ids.sender;
                cluster.add_peer(Arc::downgrade(&agent));
                self.agents.insert(Arc::clone(&agent.peer), (agent, 1));
                return None;
            }

            // No FinishCluster job exists, which means that the cluster is still active and thus
            // contains more ids than this peer sent us. Assuming that the remaining ids will be
            // accepted, we emit the peer as useless. The peer will eventually be upgraded to useful
            // if the assumption doesn't hold.
            return Some(epoch_ids.sender);
        }

        epoch_ids.ids = epoch_ids.ids.split_off(num_ids_to_remove);
        epoch_ids.first_epoch_number += num_ids_to_remove;

        // ----------

        let mut id_index = 0;
        let mut new_clusters = VecDeque::new();
        let mut num_clusters = 0;

        let checkpoint_epoch = epoch_ids.get_checkpoint_epoch();
        let agent = epoch_ids.sender;

        debug!(
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
                && cluster.first_epoch_number + cluster.epoch_ids.len()
                    > epoch_ids.first_epoch_number
            {
                // Compare epoch ids in the overlapping region.
                let start_offset = epoch_ids.first_epoch_number - cluster.first_epoch_number;
                let len = usize::min(
                    cluster.epoch_ids.len() - start_offset,
                    epoch_ids.ids.len() - id_index,
                );
                let match_until = cluster.epoch_ids[start_offset..start_offset + len]
                    .iter()
                    .zip(&epoch_ids.ids[id_index..id_index + len])
                    .position(|(first, second)| first != second)
                    .unwrap_or(len);

                debug!(
                    "Comparing with cluster #{}: first_epoch_number={}, num_ids={}, match_until={}",
                    cluster.id,
                    cluster.first_epoch_number,
                    cluster.epoch_ids.len(),
                    match_until
                );

                // If there is no match at all, skip to the next cluster.
                if match_until > 0 {
                    // If there is only a partial match, split the current cluster. The current cluster
                    // is truncated to the matching overlapping part and the removed ids are put in a new
                    // cluster. Buffer up the new clusters and insert them after we finish iterating over
                    // sync_clusters.
                    if match_until < cluster.epoch_ids.len() - start_offset {
                        // If the cluster to be split has already been processed past the splitting
                        // point, skip the matched ids without adding the peer to the cluster.
                        let split_at = start_offset + match_until;
                        if cluster.num_epochs_finished() > split_at {
                            debug!(
                                "Ignoring {} ids already processed in cluster #{}, {} ids remaining",
                                match_until,
                                cluster.id,
                                epoch_ids.ids.len().saturating_sub(id_index)
                            );

                            id_index += match_until;
                            if id_index >= epoch_ids.ids.len() {
                                break;
                            } else {
                                continue;
                            }
                        }

                        debug!(
                            "Splitting cluster #{}: start_offset={}, split_at={} {:#?}",
                            cluster.id, start_offset, split_at, cluster,
                        );
                        new_clusters.push_back(cluster.split_off(split_at));
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
                    && cluster.epoch_ids[0] == checkpoint_id
                    && cluster.epoch_ids.len() == 1
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
            debug!("Adding new cluster: {:#?}", cluster);
            for agent in cluster.peers() {
                if let Some(agent) = Weak::upgrade(agent) {
                    let pair = self.agents.get_mut(&agent.peer).unwrap_or_else(|| {
                        panic!(
                            "Agent should be present {:?} cluster {}",
                            agent.peer.id(),
                            cluster.id
                        )
                    });
                    pair.1 += 1;
                }
            }
        }

        // Add buffered clusters to sync_clusters.
        self.epoch_clusters.append(&mut new_clusters);

        None
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

        debug!("Syncing cluster {} at index {} out of {} clusters: current_epoch={}, first_epoch_number={}, num_ids={}, num_peers: {}",
               best_cluster.id, best_idx, clusters.len() + 1, current_epoch, best_cluster.first_epoch_number, best_cluster.epoch_ids.len(), best_cluster.peers().len());

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
        cluster: SyncCluster<TNetwork::PeerType>,
        result: SyncClusterResult,
    ) {
        if result != SyncClusterResult::NoMoreEpochs {
            debug!(
                "Failed to push epoch from cluster {}: {:?}",
                cluster.id, result
            );
        }

        // Decrement the cluster count for all peers in the cluster.
        for peer in cluster.peers() {
            if let Some(agent) = Weak::upgrade(peer) {
                let cluster_count = {
                    let pair = self.agents.get_mut(&agent.peer).unwrap_or_else(|| {
                        panic!(
                            "Agent should be present {:?} cluster {}",
                            agent.peer.id(),
                            cluster.id
                        )
                    });
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
                        debug!(
                            "Closing connection to peer {:?} after cluster {} failed",
                            agent.peer.id(),
                            cluster.id
                        );
                        //agent.peer.close(CloseReason::Other);
                    }
                }
            }
        }
    }
}
