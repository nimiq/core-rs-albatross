use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::{Arc, Weak};

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::task::{Context, Poll};
use futures::{FutureExt, Stream, StreamExt};
use parking_lot::RwLock;
use tokio::task::spawn_blocking;
use tokio_stream::wrappers::BroadcastStream;

use nimiq_block::Block;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::prelude::{CloseReason, Network, NetworkEvent, Peer};
use nimiq_primitives::policy;

use crate::consensus_agent::ConsensusAgent;
use crate::messages::{BlockHashType, RequestBlockHashesFilter};
use crate::sync::history::cluster::{SyncCluster, SyncClusterResult};
use crate::sync::request_component::HistorySyncStream;

struct EpochIds<TPeer: Peer> {
    on_same_chain: bool,
    ids: Vec<Blake2bHash>,
    checkpoint_id: Option<Blake2bHash>, // The most recent checkpoint block in the latest epoch.
    first_epoch_number: usize,
    sender: Arc<ConsensusAgent<TPeer>>,
}

impl<TPeer: Peer> Clone for EpochIds<TPeer> {
    fn clone(&self) -> Self {
        EpochIds {
            on_same_chain: self.on_same_chain,
            ids: self.ids.clone(),
            checkpoint_id: self.checkpoint_id.clone(),
            first_epoch_number: self.first_epoch_number,
            sender: Arc::clone(&self.sender),
        }
    }
}

impl<TPeer: Peer> EpochIds<TPeer> {
    fn get_checkpoint_epoch(&self) -> usize {
        self.first_epoch_number + self.ids.len()
    }
}

pub struct HistorySync<TNetwork: Network> {
    blockchain: Arc<RwLock<Blockchain>>,
    network_event_rx: BroadcastStream<NetworkEvent<TNetwork::PeerType>>,
    epoch_ids_stream: FuturesUnordered<BoxFuture<'static, Option<EpochIds<TNetwork::PeerType>>>>,
    epoch_clusters: VecDeque<SyncCluster<TNetwork::PeerType>>,
    active_epoch_cluster: Option<SyncCluster<TNetwork::PeerType>>,
    checkpoint_clusters: VecDeque<SyncCluster<TNetwork::PeerType>>,
    active_checkpoint_cluster: Option<SyncCluster<TNetwork::PeerType>>,
    agents: HashMap<Arc<TNetwork::PeerType>, (Arc<ConsensusAgent<TNetwork::PeerType>>, usize)>,
    push_epoch_future: Option<BoxFuture<'static, SyncClusterResult>>,
    push_checkpoint_future: Option<BoxFuture<'static, SyncClusterResult>>,
}

impl<TNetwork: Network> HistorySync<TNetwork> {
    const MAX_CLUSTERS: usize = 100;

    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network_event_rx: BroadcastStream<NetworkEvent<TNetwork::PeerType>>,
    ) -> Self {
        Self {
            blockchain,
            network_event_rx,
            epoch_ids_stream: FuturesUnordered::new(),
            epoch_clusters: VecDeque::new(),
            active_epoch_cluster: None,
            checkpoint_clusters: VecDeque::new(),
            active_checkpoint_cluster: None,
            agents: HashMap::new(),
            push_epoch_future: None,
            push_checkpoint_future: None,
        }
    }

    pub fn agents(&self) -> impl Iterator<Item = &Arc<ConsensusAgent<TNetwork::PeerType>>> {
        self.agents.values().map(|(agent, _)| agent)
    }

    async fn request_epoch_ids(
        blockchain: Arc<RwLock<Blockchain>>,
        agent: Arc<ConsensusAgent<TNetwork::PeerType>>,
    ) -> Option<EpochIds<TNetwork::PeerType>> {
        let (locators, epoch_number) = {
            // Order matters here. The first hash found by the recipient of the request  will be used, so they need to be
            // in backwards block height order.
            let blockchain = blockchain.read();
            let election_head = blockchain.election_head();
            let macro_head = blockchain.macro_head();

            // So if there is a checkpoint hash that should be included in addition to the election block hash, it should come first.
            let mut locators = vec![];
            if macro_head.hash() != election_head.hash() {
                locators.push(macro_head.hash());
            }
            // The election bock is at the end here
            locators.push(election_head.hash());

            (
                locators,
                policy::epoch_at(election_head.header.block_number),
            )
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
                        on_same_chain: false,
                        ids: Vec::new(),
                        checkpoint_id: None,
                        first_epoch_number: 0,
                        sender: agent,
                    });
                }

                let hashes = block_hashes.hashes.unwrap();

                // Get checkpoint id if exists.
                let checkpoint_id = hashes.last().and_then(|(ty, id)| {
                    if *ty == BlockHashType::Checkpoint {
                        Some(id.clone())
                    } else {
                        None
                    }
                });
                // Filter checkpoint from block hashes and map to hash.
                let epoch_ids = hashes
                    .into_iter()
                    .filter_map(|(ty, id)| {
                        if ty == BlockHashType::Election {
                            Some(id)
                        } else {
                            None
                        }
                    })
                    .collect();
                Some(EpochIds {
                    on_same_chain: true,
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

    fn cluster_epoch_ids(&mut self, mut epoch_ids: EpochIds<TNetwork::PeerType>) {
        if !epoch_ids.on_same_chain {
            return;
        }

        let checkpoint_epoch = epoch_ids.get_checkpoint_epoch();
        let agent = epoch_ids.sender;

        let (current_id, election_head) = {
            let blockchain = self.blockchain.read();
            (blockchain.election_head_hash(), blockchain.election_head())
        };

        // If `epoch_ids` includes known blocks, truncate (or discard on fork prior to our accepted state).
        let current_epoch = policy::epoch_at(election_head.header.block_number) as usize;
        if !epoch_ids.ids.is_empty() && epoch_ids.first_epoch_number <= current_epoch {
            // Check most recent id against our state.
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
            self.active_epoch_cluster.is_some(),
        );

        let epoch_clusters = self
            .epoch_clusters
            .iter_mut()
            .chain(self.active_epoch_cluster.iter_mut());
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
            // We do not increment the num_clusters here, as this is done in the loop later on.
        }

        // Now cluster the checkpoint id if present.
        if let Some(checkpoint_id) = epoch_ids.checkpoint_id {
            let mut found_cluster = false;
            let checkpoint_clusters = self
                .checkpoint_clusters
                .iter_mut()
                .chain(self.active_checkpoint_cluster.iter_mut());
            for cluster in checkpoint_clusters {
                // Currently, we do not need to remove old checkpoint ids from the same peer.
                // Since we only request new epoch ids (and checkpoints) once a peer has 0 clusters,
                // we can never receive an updated checkpoint.
                // When this invariant changes, we need to remove old checkpoints of that peer here!

                // Look for clusters at the same epoch with the same hash.
                if cluster.first_epoch_number == checkpoint_epoch && cluster.ids[0] == checkpoint_id
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

    fn find_best_epoch_cluster(&mut self) -> Option<SyncCluster<TNetwork::PeerType>> {
        HistorySync::<TNetwork>::find_best_cluster(&mut self.epoch_clusters, &self.blockchain)
    }

    fn find_best_checkpoint_cluster(&mut self) -> Option<SyncCluster<TNetwork::PeerType>> {
        HistorySync::<TNetwork>::find_best_cluster(&mut self.checkpoint_clusters, &self.blockchain)
    }

    fn find_best_cluster(
        clusters: &mut VecDeque<SyncCluster<TNetwork::PeerType>>,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) -> Option<SyncCluster<TNetwork::PeerType>> {
        if clusters.is_empty() {
            return None;
        }

        let current_epoch =
            policy::epoch_at(blockchain.read().election_head().header.block_number) as usize;

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
    /// If for any given peer the cluster count falls to zero and `request_more_epochs` is true,
    /// a request for more epoch ids will be send to the peer.
    ///
    /// Peers with no clusters are always removed from the agent set as they are re added if they
    /// provide new epoch ids or emitted as synced peers if there are no new ids to sync.
    fn finish_cluster(
        &mut self,
        cluster: &SyncCluster<<TNetwork as Network>::PeerType>,
        request_more_epochs: bool,
        cx: &mut Context<'_>,
    ) {
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

                    if request_more_epochs {
                        self.add_agent(agent);
                        // Pushing the future to FuturesUnordered above does not wake the task that
                        // polls `epoch_ids_stream`. Therefore, we need to wake the task manually.
                        cx.waker().wake_by_ref();
                    } else {
                        // FIXME: Disconnect peer
                        // agent.peer.close()
                    }
                }
            }
        }
    }

    fn sync_epochs(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if self.push_epoch_future.is_none() {
            // Initialize active_epoch_cluster if there is none.
            if self.active_epoch_cluster.is_none() {
                self.active_epoch_cluster = self.find_best_epoch_cluster();
            }

            // Poll the best epoch cluster.
            if let Some(best_cluster) = self.active_epoch_cluster.as_mut() {
                let cluster_res = ready!(best_cluster.poll_next_unpin(cx));
                let blockchain = self.blockchain.clone();

                let future = async move {
                    match cluster_res {
                        Some(Ok(epoch)) => {
                            debug!(
                                "Processing epoch #{} ({} history items)",
                                epoch.block.epoch_number(),
                                epoch.history.len()
                            );
                            let push_result = spawn_blocking(move || {
                                Blockchain::push_history_sync(
                                    blockchain.upgradable_read(),
                                    Block::Macro(epoch.block),
                                    &epoch.history,
                                )
                            })
                            .await
                            .expect("blockchain.push_history_sync() should not panic");
                            SyncClusterResult::from(push_result)
                        }
                        Some(Err(e)) => {
                            log::debug!("Polling the best SyncCluster returned an error: {:?}", e);
                            SyncClusterResult::Error
                        }
                        None => SyncClusterResult::NoMoreEpochs,
                    }
                }
                .boxed();

                self.push_epoch_future = Some(future);
            }
        }

        if let Some(op) = self.push_epoch_future.as_mut() {
            let result = ready!(op.poll_unpin(cx));
            self.push_epoch_future = None;

            // If the epoch was successful, the cluster is not done yet
            // and we update the remaining clusters.
            if result == SyncClusterResult::EpochSuccessful {
                let best_cluster = self
                    .active_epoch_cluster
                    .as_mut()
                    .expect("active_epoch_cluster should be set");
                best_cluster.adopted_batch_set = true;
            } else {
                if result != SyncClusterResult::NoMoreEpochs {
                    debug!("Failed to push epoch: {:?}", result);
                }

                // TODO Do we really want to evict outdated clusters as well?
                let best_cluster = self
                    .active_epoch_cluster
                    .take()
                    .expect("active_epoch_cluster should be set");

                // Decrement the cluster count for all peers in the evicted cluster.
                self.finish_cluster(
                    &best_cluster,
                    result != SyncClusterResult::Error && best_cluster.adopted_batch_set,
                    cx,
                );

                // Evict current best cluster and move to next one.
                self.active_epoch_cluster = self.find_best_epoch_cluster();
            }
        }
        Poll::Ready(())
    }

    fn sync_batches(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if self.push_checkpoint_future.is_none() {
            // When no more epochs are to be processed, we continue with checkpoint blocks.
            // Poll the best checkpoint cluster.
            let current_epoch = self.blockchain.read().epoch_number() as usize;

            // Initialize active_checkpoint_cluster if there is none.
            if self.active_checkpoint_cluster.is_none() {
                self.active_checkpoint_cluster = self.find_best_checkpoint_cluster();
            }

            if let Some(best_cluster) = self.active_checkpoint_cluster.as_mut() {
                let blockchain = self.blockchain.clone();
                let cluster_res = if best_cluster.first_epoch_number > current_epoch {
                    ready!(best_cluster.poll_next_unpin(cx))
                } else {
                    None
                };

                let future = async move {
                    match cluster_res {
                        Some(Ok(batch)) => {
                            debug!(
                                "Processing partial epoch #{} ({} history items)",
                                batch.block.epoch_number(),
                                batch.history.len()
                            );
                            let push_result = spawn_blocking(move || {
                                Blockchain::push_history_sync(
                                    blockchain.upgradable_read(),
                                    Block::Macro(batch.block),
                                    &batch.history,
                                )
                            })
                            .await
                            .expect("blockchain.push_history_sync() should not panic");
                            SyncClusterResult::from(push_result)
                        }
                        Some(Err(e)) => e,
                        None => SyncClusterResult::NoMoreEpochs,
                    }
                }
                .boxed();

                self.push_checkpoint_future = Some(future);
            }
        }

        if let Some(op) = self.push_checkpoint_future.as_mut() {
            let result = ready!(op.poll_unpin(cx));
            self.push_checkpoint_future = None;

            if result == SyncClusterResult::Error || result == SyncClusterResult::Outdated {
                debug!("Failed to push checkpoint: {:?}", result);
            }

            // Since checkpoint clusters are always of length 1, we can remove them immediately.
            let best_cluster = self
                .active_checkpoint_cluster
                .take()
                .expect("active_checkpoint_cluster should be set");

            // Decrement the cluster count for all peers in the evicted cluster.
            self.finish_cluster(&best_cluster, result != SyncClusterResult::Error, cx);

            // Move to next cluster.
            self.active_checkpoint_cluster = self.find_best_checkpoint_cluster();
        }
        Poll::Ready(())
    }
}

impl<TNetwork: Network> Stream for HistorySync<TNetwork> {
    type Item = Arc<ConsensusAgent<TNetwork::PeerType>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            match result {
                Ok(NetworkEvent::PeerLeft(peer)) => {
                    // Delete the ConsensusAgent from the agents map, removing the only "persistent"
                    // strong reference to it. There might not be an entry for every peer (e.g. if
                    // it didn't send any epoch ids).
                    self.agents.remove(&peer);
                }
                Ok(NetworkEvent::PeerJoined(peer)) => {
                    // Create a ConsensusAgent for the peer that joined and request epoch_ids from it.
                    let agent = Arc::new(ConsensusAgent::new(peer));
                    self.add_agent(agent);
                }
                Err(_) => return Poll::Ready(None),
            }
        }

        // Stop pulling in new EpochIds if we hit a maximum a number of clusters to prevent DoS.
        loop {
            if self.epoch_clusters.len() >= Self::MAX_CLUSTERS {
                // TODO: We still want to get the wakes for the epoch_ids_stream
                //  even if we don't poll it now.
                break;
            }

            if let Poll::Ready(Some(epoch_ids)) = self.epoch_ids_stream.poll_next_unpin(cx) {
                if let Some(epoch_ids) = epoch_ids {
                    // The peer might have disconnected during the request.
                    // FIXME Check if the peer is still connected

                    if !epoch_ids.on_same_chain {
                        debug!(
                            "Peer is on different chain: {:?}",
                            epoch_ids.sender.peer.id()
                        );
                        // TODO: Send further locators. Possibly find branching point of fork.
                    } else if epoch_ids.ids.is_empty() && epoch_ids.checkpoint_id.is_none() {
                        // We are synced with this peer.
                        debug!(
                            "Peer has finished syncing: {:?}",
                            epoch_ids.sender.peer.id()
                        );
                        return Poll::Ready(Some(epoch_ids.sender));
                    }
                    self.cluster_epoch_ids(epoch_ids);
                }
            } else {
                break;
            }
        }

        ready!(self.sync_epochs(cx));

        ready!(self.sync_batches(cx));

        Poll::Pending
    }
}

impl<TNetwork: Network> HistorySyncStream<TNetwork::PeerType> for HistorySync<TNetwork> {
    fn add_agent(&self, agent: Arc<ConsensusAgent<TNetwork::PeerType>>) {
        trace!("Requesting more epoch ids for peer: {:?}", agent.peer.id());
        let future = Self::request_epoch_ids(Arc::clone(&self.blockchain), agent).boxed();
        self.epoch_ids_stream.push(future);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::RwLock;

    use nimiq_blockchain::Blockchain;
    use nimiq_database::volatile::VolatileEnvironment;
    use nimiq_hash::Blake2bHash;
    use nimiq_network_interface::prelude::Network;
    use nimiq_network_mock::{MockHub, MockNetwork, MockPeer};
    use nimiq_primitives::networks::NetworkId;
    use nimiq_utils::time::OffsetTime;

    use crate::consensus_agent::ConsensusAgent;
    use crate::sync::history::sync::EpochIds;
    use crate::sync::history::HistorySync;

    #[tokio::test]
    async fn it_can_cluster_epoch_ids() {
        fn generate_epoch_ids(
            agent: &Arc<ConsensusAgent<MockPeer>>,
            len: usize,
            first_epoch_number: usize,
            diverge_at: Option<usize>,
        ) -> EpochIds<MockPeer> {
            let mut ids = vec![];
            for i in first_epoch_number..first_epoch_number + len {
                let mut epoch_id = [0u8; 32];
                epoch_id[0..8].copy_from_slice(&i.to_le_bytes());

                if diverge_at
                    .map(|d| i >= d + first_epoch_number)
                    .unwrap_or(false)
                {
                    epoch_id[9] = 1;
                }

                ids.push(Blake2bHash::from(epoch_id));
            }

            EpochIds {
                on_same_chain: true,
                ids,
                checkpoint_id: None,
                first_epoch_number,
                sender: Arc::clone(agent),
            }
        }

        let time = Arc::new(OffsetTime::new());
        let env1 = VolatileEnvironment::new(10).unwrap();
        let blockchain = Arc::new(RwLock::new(
            Blockchain::new(env1, NetworkId::UnitAlbatross, time).unwrap(),
        ));

        let mut hub = MockHub::default();

        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let net3 = Arc::new(hub.new_network());
        net1.dial_mock(&net2);
        net1.dial_mock(&net3);
        let peers = net1.get_peers();
        let consensus_agents: Vec<_> = peers
            .into_iter()
            .map(ConsensusAgent::new)
            .map(Arc::new)
            .collect();

        fn run_test<F>(
            blockchain: &Arc<RwLock<Blockchain>>,
            net: &Arc<MockNetwork>,
            epoch_ids1: EpochIds<MockPeer>,
            epoch_ids2: EpochIds<MockPeer>,
            test: F,
            symmetric: bool,
        ) where
            F: Fn(HistorySync<MockNetwork>),
        {
            let mut sync =
                HistorySync::<MockNetwork>::new(Arc::clone(blockchain), net.subscribe_events());
            sync.cluster_epoch_ids(epoch_ids1.clone());
            sync.cluster_epoch_ids(epoch_ids2.clone());
            test(sync);

            // Symmetric check
            if symmetric {
                let mut sync =
                    HistorySync::<MockNetwork>::new(Arc::clone(blockchain), net.subscribe_events());
                sync.cluster_epoch_ids(epoch_ids2);
                sync.cluster_epoch_ids(epoch_ids1);
                test(sync);
            }
        }

        // This test tests several aspects of the epoch id clustering.
        // 1) identical epoch ids
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 1, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 1);
                assert_eq!(sync.epoch_clusters[0].ids.len(), 10);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 2);
            },
            true,
        );

        // 2) disjoint epoch ids
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 1, Some(0));
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 2);
                assert_eq!(sync.epoch_clusters[0].ids.len(), 10);
                assert_eq!(sync.epoch_clusters[1].ids.len(), 10);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[1].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 1);
                assert_eq!(sync.epoch_clusters[1].batch_set_queue.peers.len(), 1);
            },
            true,
        );

        // 3) same offset and history, second shorter than first
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 8, 1, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 2);
                assert_eq!(sync.epoch_clusters[0].ids.len(), 8);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 2);
                assert_eq!(sync.epoch_clusters[1].ids.len(), 2);
                assert_eq!(sync.epoch_clusters[1].first_epoch_number, 9);
                assert_eq!(sync.epoch_clusters[1].batch_set_queue.peers.len(), 1);
            },
            true,
        );

        // 4) different offset, same history, but second is longer
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 3, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 2);
                assert_eq!(sync.epoch_clusters[0].ids.len(), 10);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 2);
                assert_eq!(sync.epoch_clusters[1].ids.len(), 2);
                assert_eq!(sync.epoch_clusters[1].first_epoch_number, 11);
                assert_eq!(sync.epoch_clusters[1].batch_set_queue.peers.len(), 1);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change

        // 5) Irrelevant epoch ids (that would constitute forks from what we have already seen.
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 0, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 0, Some(0));
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 0);
            },
            true,
        );

        // 6) different offset, same history, but second is shorter
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 5, 3, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 2);
                assert_eq!(sync.epoch_clusters[0].ids.len(), 7);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 2);
                assert_eq!(sync.epoch_clusters[1].ids.len(), 3);
                assert_eq!(sync.epoch_clusters[1].first_epoch_number, 8);
                assert_eq!(sync.epoch_clusters[1].batch_set_queue.peers.len(), 1);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change

        // 7) different offset, diverging history, second longer
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 8, 4, Some(6));
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 3);
                assert_eq!(sync.epoch_clusters[0].ids.len(), 9);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 2);
                assert_eq!(sync.epoch_clusters[1].ids.len(), 1);
                assert_eq!(sync.epoch_clusters[1].first_epoch_number, 10);
                assert_eq!(sync.epoch_clusters[1].batch_set_queue.peers.len(), 1);
                assert_eq!(sync.epoch_clusters[2].ids.len(), 2);
                assert_eq!(sync.epoch_clusters[2].first_epoch_number, 10);
                assert_eq!(sync.epoch_clusters[2].batch_set_queue.peers.len(), 1);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change
    }
}
