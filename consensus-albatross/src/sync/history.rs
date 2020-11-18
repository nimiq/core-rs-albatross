use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::pin::Pin;
use std::sync::{Arc, Weak};

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::task::{Context, Poll};
use futures::{FutureExt, Stream, StreamExt};
use tokio::sync::broadcast;

use block_albatross::{Block, MacroBlock};
use blockchain_albatross::history_store;
use blockchain_albatross::history_store::ExtendedTransaction;
use blockchain_albatross::Blockchain;
use hash::Blake2bHash;
use network_interface::prelude::{CloseReason, Network, NetworkEvent, Peer};
use primitives::policy;

use crate::consensus_agent::ConsensusAgent;
use crate::messages::{Epoch as EpochInfo, HistoryChunk, RequestBlockHashesFilter};
use crate::sync::sync_queue::SyncQueue;

struct PendingEpoch {
    block: MacroBlock,
    history_len: usize,
    history: Vec<ExtendedTransaction>,
}
impl PendingEpoch {
    fn is_complete(&self) -> bool {
        self.history_len == self.history.len()
    }

    fn epoch_number(&self) -> u32 {
        policy::epoch_at(self.block.header.block_number)
    }
}

pub struct Epoch {
    block: MacroBlock,
    history: Vec<ExtendedTransaction>,
}

struct SyncCluster<TPeer: Peer> {
    epoch_ids: Vec<Blake2bHash>,
    epoch_offset: usize,

    epoch_queue: SyncQueue<TPeer, Blake2bHash, EpochInfo>,
    history_queue: SyncQueue<TPeer, (u32, usize), (u32, HistoryChunk)>,

    pending_epochs: VecDeque<PendingEpoch>,
}

impl<TPeer: Peer + 'static> SyncCluster<TPeer> {
    const NUM_PENDING_EPOCHS: usize = 5;
    const NUM_PENDING_CHUNKS: usize = 12;

    fn new(epoch_ids: Vec<Blake2bHash>, epoch_offset: usize, peers: Vec<Weak<ConsensusAgent<TPeer>>>) -> Self {
        let epoch_queue = SyncQueue::new(epoch_ids.clone(), peers.clone(), Self::NUM_PENDING_EPOCHS, |id, peer| {
            async move { peer.request_epoch(id).await.ok() }.boxed()
        });
        let history_queue = SyncQueue::new(
            Vec::<(u32, usize)>::new(),
            peers,
            Self::NUM_PENDING_CHUNKS,
            move |(epoch_number, chunk_index), peer| {
                async move {
                    peer.request_history_chunk(epoch_number, chunk_index)
                        .await
                        .ok()
                        .map(|chunk| (epoch_number, chunk))
                }
                .boxed()
            },
        );
        Self {
            epoch_ids,
            epoch_offset,
            epoch_queue,
            history_queue,
            pending_epochs: VecDeque::with_capacity(Self::NUM_PENDING_EPOCHS),
        }
    }

    fn on_epoch_received(&mut self, epoch: EpochInfo) {
        // TODO Verify macro blocks and their ordering

        // Queue history chunks for the given epoch for download.
        let block_number = epoch.block.header.block_number;
        let history_chunk_ids = (0..(epoch.history_len as usize / history_store::CHUNK_SIZE))
            .map(|i| (block_number, i))
            .collect();
        self.history_queue.add_ids(history_chunk_ids);

        // We keep the epoch in pending_epochs while the history is downloading.
        self.pending_epochs.push_back(PendingEpoch {
            block: epoch.block,
            history_len: epoch.history_len as usize,
            history: Vec::new(),
        });
    }

    fn on_history_chunk_received(&mut self, epoch_number: u32, history_chunk: HistoryChunk) {
        // Find epoch in pending_epochs.
        let first_epoch_number = self.pending_epochs[0].epoch_number();
        let epoch_index = (epoch_number - first_epoch_number) as usize;
        let epoch = &mut self.pending_epochs[epoch_index];

        // TODO This assumes that we have already filtered responses with no chunk.
        // Add the received history chunk to the pending epoch.
        let mut chunk = history_chunk.chunk.expect("History chunk missing").history;
        epoch.history.append(&mut chunk);
    }

    fn add_peer(&mut self, peer: Weak<ConsensusAgent<TPeer>>) {
        // TODO keep only one list of peers
        self.epoch_queue.add_peer(Weak::clone(&peer));
        self.history_queue.add_peer(peer);
    }

    fn peers(&self) -> &Vec<Weak<ConsensusAgent<TPeer>>> {
        &self.epoch_queue.peers
    }

    fn split_off(&mut self, at: usize) -> Self {
        let ids = self.epoch_ids.split_off(at);
        let offset = self.epoch_offset + at;

        // Remove the split-off ids from our epoch queue.
        self.epoch_queue.truncate_ids(at);

        Self::new(ids, offset, self.epoch_queue.peers.clone())
    }
}

impl<TPeer: Peer + 'static> Stream for SyncCluster<TPeer> {
    type Item = Result<Epoch, ()>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.pending_epochs.len() < Self::NUM_PENDING_EPOCHS {
            while let Poll::Ready(Some(result)) = self.epoch_queue.poll_next_unpin(cx) {
                match result {
                    Ok(epoch) => self.on_epoch_received(epoch),
                    Err(_) => return Poll::Ready(Some(Err(()))), // TODO Error
                }
            }
        }

        while let Poll::Ready(Some(result)) = self.history_queue.poll_next_unpin(cx) {
            match result {
                Ok((epoch_number, history_chunk)) => {
                    self.on_history_chunk_received(epoch_number, history_chunk);

                    // Emit finished epochs.
                    if self.pending_epochs[0].is_complete() {
                        let epoch = self.pending_epochs.pop_front().unwrap();
                        let epoch = Epoch {
                            block: epoch.block,
                            history: epoch.history,
                        };
                        return Poll::Ready(Some(Ok(epoch)));
                    }
                }
                Err(_) => return Poll::Ready(Some(Err(()))), // TODO Error
            }
        }

        // We're done if there are no more epochs to process.
        if self.epoch_queue.is_empty() && self.pending_epochs.is_empty() {
            return Poll::Ready(None);
        }

        Poll::Pending
    }
}

impl<TPeer: Peer> PartialEq for SyncCluster<TPeer> {
    fn eq(&self, other: &Self) -> bool {
        self.epoch_offset == other.epoch_offset && self.epoch_queue.num_peers() == other.epoch_queue.num_peers() && self.epoch_ids == other.epoch_ids
    }
}
impl<TPeer: Peer> Eq for SyncCluster<TPeer> {}
impl<TPeer: Peer> PartialOrd for SyncCluster<TPeer> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}
impl<TPeer: Peer> Ord for SyncCluster<TPeer> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.epoch_offset
            .cmp(&other.epoch_offset) // Lower offset first
            .then_with(|| other.epoch_queue.num_peers().cmp(&self.epoch_queue.num_peers())) // Higher peer count first
            .then_with(|| other.epoch_ids.len().cmp(&self.epoch_ids.len())) // More ids first
            .then_with(|| self.epoch_ids.cmp(&other.epoch_ids)) //
            .reverse() // We want the best cluster to be *last*
    }
}

struct EpochIds<TPeer: Peer> {
    ids: Vec<Blake2bHash>,
    offset: usize,
    sender: Arc<ConsensusAgent<TPeer>>,
}

impl<TPeer: Peer> Clone for EpochIds<TPeer> {
    fn clone(&self) -> Self {
        EpochIds {
            ids: self.ids.clone(),
            offset: self.offset,
            sender: Arc::clone(&self.sender),
        }
    }
}

struct HistorySync<TNetwork: Network> {
    blockchain: Arc<Blockchain>,
    network_event_rx: broadcast::Receiver<NetworkEvent<TNetwork::PeerType>>,
    epoch_ids_stream: FuturesUnordered<BoxFuture<'static, Option<EpochIds<TNetwork::PeerType>>>>,
    sync_clusters: Vec<SyncCluster<TNetwork::PeerType>>,
    agents: HashMap<Arc<TNetwork::PeerType>, (Arc<ConsensusAgent<TNetwork::PeerType>>, usize)>,
}

impl<TNetwork: Network> HistorySync<TNetwork> {
    const MAX_CLUSTERS: usize = 100;

    pub fn new(blockchain: Arc<Blockchain>, network_event_rx: broadcast::Receiver<NetworkEvent<TNetwork::PeerType>>) -> Self {
        Self {
            blockchain,
            network_event_rx,
            epoch_ids_stream: FuturesUnordered::new(),
            sync_clusters: Vec::new(),
            agents: HashMap::new(),
        }
    }

    async fn request_epoch_ids(blockchain: Arc<Blockchain>, agent: Arc<ConsensusAgent<TNetwork::PeerType>>) -> Option<EpochIds<TNetwork::PeerType>> {
        let (locator, epoch_number) = {
            let election_head = blockchain.election_head();
            (election_head.hash(), policy::epoch_at(election_head.header.block_number))
        };

        let result = agent
            .request_block_hashes(
                vec![locator],
                1000, // TODO: Use other value
                RequestBlockHashesFilter::ElectionOnly,
            )
            .await;

        match result {
            Ok(block_hashes) => Some(EpochIds {
                ids: block_hashes.hashes,
                offset: epoch_number as usize + 1,
                sender: agent,
            }),
            Err(_) => {
                agent.peer.close(CloseReason::Other).await;
                None
            }
        }
    }

    fn cluster_epoch_ids(&mut self, mut epoch_ids: EpochIds<TNetwork::PeerType>) {
        let agent = epoch_ids.sender;

        // Truncate beginning of cluster to our current blockchain state.
        let current_id = self.blockchain.election_head_hash();
        let current_offset = policy::epoch_at(self.blockchain.election_head().header.block_number) as usize;
        // If `epoch_ids` includes known blocks, truncate (or discard on fork prior to our accepted state).
        if epoch_ids.offset <= current_offset {
            // Check most recent id against our state.
            if current_id == epoch_ids.ids[current_offset - epoch_ids.offset] {
                // Remove known blocks.
                epoch_ids.ids = epoch_ids.ids.split_off(current_offset - epoch_ids.offset + 1);
                epoch_ids.offset = current_offset;

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
        let mut new_clusters = Vec::new();
        let mut num_clusters = 0;

        for cluster in &mut self.sync_clusters {
            // Check if given epoch_ids and the current cluster potentially overlap.
            if cluster.epoch_offset <= epoch_ids.offset && cluster.epoch_offset + cluster.epoch_ids.len() > epoch_ids.offset {
                // Compare ids in the overlapping region.
                let start_offset = epoch_ids.offset - cluster.epoch_offset;
                let len = usize::min(cluster.epoch_ids.len() - start_offset, epoch_ids.ids.len() - id_index);
                let match_until = cluster.epoch_ids[start_offset..start_offset + len]
                    .iter()
                    .zip(&epoch_ids.ids[id_index..id_index + len])
                    .position(|(first, second)| first != second)
                    .unwrap_or(len);

                // If there is no match at all, skip to the next cluster.
                if match_until > 0 {
                    // If there is only a partial match, split the current cluster. The current cluster
                    // is truncated to the matching overlapping part and the removed ids are put in a new
                    // cluster. Buffer up the new clusters and insert them after we finish iterating over
                    // sync_clusters.
                    if match_until < cluster.epoch_ids.len() - start_offset {
                        new_clusters.push(cluster.split_off(start_offset + match_until));
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
            new_clusters.push(SyncCluster::new(
                Vec::from(&epoch_ids.ids[id_index..]),
                epoch_ids.offset + id_index,
                vec![Arc::downgrade(&agent)],
            ));
        }

        // Store agent Arc and number of clusters it's in.
        self.agents.insert(Arc::clone(&agent.peer), (agent, num_clusters));

        // Update cluster counts for all peers in new clusters.
        for cluster in &new_clusters {
            for agent in cluster.peers() {
                if let Some(agent) = Weak::upgrade(agent) {
                    let pair = self.agents.get_mut(&agent.peer).expect("Agent should be present");
                    pair.1 += 1;
                }
            }
        }

        // Add buffered clusters to sync_clusters and sort it.
        self.sync_clusters.append(&mut new_clusters);
        self.sync_clusters.sort();
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
                    let future = Self::request_epoch_ids(Arc::clone(&self.blockchain), agent).boxed();
                    self.epoch_ids_stream.push(future);
                }
                Err(_) => return Poll::Ready(None),
            }
        }

        // Stop pulling in new EpochIds if we hit a maximum a number of clusters to prevent DoS.
        loop {
            if self.sync_clusters.len() >= Self::MAX_CLUSTERS {
                break;
            }

            if let Poll::Ready(Some(epoch_ids)) = self.epoch_ids_stream.poll_next_unpin(cx) {
                if let Some(epoch_ids) = epoch_ids {
                    // The peer might have disconnected during the request.
                    // FIXME Check if the peer is still connected

                    // FIXME We want to distinguish between "locator hash not found" (i.e. peer is
                    // on a different chain) and "no more hashes past locator" (we are in sync with
                    // the peer).
                    if epoch_ids.ids.is_empty() {
                        // We are synced with this peer.
                        return Poll::Ready(Some(epoch_ids.sender));
                    }
                    self.cluster_epoch_ids(epoch_ids);
                }
            } else {
                break;
            }
        }

        // Poll the best cluster.
        // The best cluster is the last element in sync_clusters, so removing it is cheap.
        while !self.sync_clusters.is_empty() {
            let best_cluster = self.sync_clusters.last_mut().expect("sync_clusters no empty");
            let result = match ready!(best_cluster.poll_next_unpin(cx)) {
                Some(Ok(epoch)) => Some(self.blockchain.push_history_sync(Block::Macro(epoch.block), &epoch.history).map_err(|_| ())),
                Some(Err(_)) => Some(Err(())),
                None => None,
            };

            let cluster_synced = result.is_none();
            if cluster_synced || result.unwrap().is_err() {
                // Evict current best cluster and move to next one.
                let cluster = self.sync_clusters.pop().expect("sync_clusters not empty");

                // TODO Cut off the ids we have already adopted from the start of the next cluster. Remove empty clusters.

                // Decrement the cluster count for all peers in the evicted cluster.
                for peer in cluster.peers() {
                    if let Some(agent) = Weak::upgrade(peer) {
                        let cluster_count = {
                            let pair = self.agents.get_mut(&agent.peer).expect("Agent should be present");
                            pair.1 -= 1;
                            pair.1
                        };

                        // If the peer isn't in any more clusters, request more epoch_ids from it.
                        // Only do so if the cluster was synced.
                        if cluster_count == 0 {
                            // Always remove agent from agents map. It will be re-added if it returns more
                            // epoch_ids and dropped otherwise.
                            self.agents.remove(&agent.peer);

                            if cluster_synced {
                                let future = Self::request_epoch_ids(Arc::clone(&self.blockchain), agent).boxed();
                                self.epoch_ids_stream.push(future);
                            }
                        }
                    }
                }
            }
        }

        return Poll::Pending;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use nimiq_database::volatile::VolatileEnvironment;
    use nimiq_genesis::NetworkId;
    use nimiq_network_mock::network::{MockNetwork, MockPeer};

    #[tokio::test]
    async fn it_can_cluster_epoch_ids() {
        fn generate_epoch_ids(agent: &Arc<ConsensusAgent<MockPeer>>, len: usize, offset: usize, diverge_at: Option<usize>) -> EpochIds<MockPeer> {
            let mut ids = vec![];
            for i in offset..offset + len {
                let mut epoch_id = [0u8; 32];
                epoch_id[0..8].copy_from_slice(&i.to_le_bytes());

                if diverge_at.map(|d| i >= d + offset).unwrap_or(false) {
                    epoch_id[9] = 1;
                }

                ids.push(Blake2bHash::from(epoch_id));
            }

            EpochIds {
                ids,
                offset,
                sender: Arc::clone(&agent),
            }
        }

        let env1 = VolatileEnvironment::new(10).unwrap();
        let blockchain = Arc::new(Blockchain::new(env1.clone(), NetworkId::UnitAlbatross).unwrap());

        let net1 = Arc::new(MockNetwork::new(1));
        let net2 = Arc::new(MockNetwork::new(2));
        let net3 = Arc::new(MockNetwork::new(3));
        net1.connect(&net2);
        net1.connect(&net3);
        let peers = net1.get_peers().await;
        let consensus_agents: Vec<_> = peers.into_iter().map(ConsensusAgent::new).map(Arc::new).collect();

        fn run_test<F>(
            blockchain: &Arc<Blockchain>,
            net: &Arc<MockNetwork>,
            epoch_ids1: EpochIds<MockPeer>,
            epoch_ids2: EpochIds<MockPeer>,
            test: F,
            symmetric: bool,
        ) where
            F: Fn(HistorySync<MockNetwork>) -> (),
        {
            let mut sync = HistorySync::<MockNetwork>::new(Arc::clone(&blockchain), net.subscribe_events());
            sync.cluster_epoch_ids(epoch_ids1.clone());
            sync.cluster_epoch_ids(epoch_ids2.clone());
            test(sync);

            // Symmetric check
            if symmetric {
                let mut sync = HistorySync::<MockNetwork>::new(Arc::clone(&blockchain), net.subscribe_events());
                sync.cluster_epoch_ids(epoch_ids2);
                sync.cluster_epoch_ids(epoch_ids1);
                test(sync);
            }
        }

        // This test tests several aspects of the epoch id clustering.
        // 1) disjunct epoch ids
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 1, Some(0));
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.sync_clusters.len(), 2);
                assert_eq!(sync.sync_clusters[0].epoch_ids.len(), 10);
                assert_eq!(sync.sync_clusters[1].epoch_ids.len(), 10);
                assert_eq!(sync.sync_clusters[0].epoch_offset, 1);
                assert_eq!(sync.sync_clusters[1].epoch_offset, 1);
                assert_eq!(sync.sync_clusters[0].epoch_queue.peers.len(), 1);
                assert_eq!(sync.sync_clusters[1].epoch_queue.peers.len(), 1);
            },
            true,
        );

        // 2) same offset and history, second shorter than first
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 8, 1, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.sync_clusters.len(), 2);
                assert_eq!(sync.sync_clusters[0].epoch_ids.len(), 2);
                assert_eq!(sync.sync_clusters[0].epoch_offset, 9);
                assert_eq!(sync.sync_clusters[0].epoch_queue.peers.len(), 1);
                assert_eq!(sync.sync_clusters[1].epoch_ids.len(), 8);
                assert_eq!(sync.sync_clusters[1].epoch_offset, 1);
                assert_eq!(sync.sync_clusters[1].epoch_queue.peers.len(), 2);
            },
            true,
        );

        // 3) different offset, same history, but second is longer
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[0], 10, 3, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.sync_clusters.len(), 2);
                assert_eq!(sync.sync_clusters[0].epoch_ids.len(), 2);
                assert_eq!(sync.sync_clusters[0].epoch_offset, 11);
                assert_eq!(sync.sync_clusters[0].epoch_queue.peers.len(), 1);
                assert_eq!(sync.sync_clusters[1].epoch_ids.len(), 10);
                assert_eq!(sync.sync_clusters[1].epoch_offset, 1);
                assert_eq!(sync.sync_clusters[1].epoch_queue.peers.len(), 2);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change

        // 4) Irrelevant epoch ids (that would constitute forks from what we have already seen.
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 0, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 0, Some(0));
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.sync_clusters.len(), 0);
            },
            true,
        );

        // 5) different offset, same history, but second is shorter
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[0], 5, 3, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.sync_clusters.len(), 2);
                assert_eq!(sync.sync_clusters[0].epoch_ids.len(), 3);
                assert_eq!(sync.sync_clusters[0].epoch_offset, 8);
                assert_eq!(sync.sync_clusters[0].epoch_queue.peers.len(), 1);
                assert_eq!(sync.sync_clusters[1].epoch_ids.len(), 7);
                assert_eq!(sync.sync_clusters[1].epoch_offset, 1);
                assert_eq!(sync.sync_clusters[1].epoch_queue.peers.len(), 2);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change

        // 6) different offset, diverging history, second longer
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[0], 8, 4, Some(6));
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.sync_clusters.len(), 3);
                assert_eq!(sync.sync_clusters[0].epoch_ids.len(), 1);
                assert_eq!(sync.sync_clusters[0].epoch_offset, 10);
                assert_eq!(sync.sync_clusters[0].epoch_queue.peers.len(), 1);
                assert_eq!(sync.sync_clusters[1].epoch_ids.len(), 2);
                assert_eq!(sync.sync_clusters[1].epoch_offset, 10);
                assert_eq!(sync.sync_clusters[1].epoch_queue.peers.len(), 1);
                assert_eq!(sync.sync_clusters[2].epoch_ids.len(), 9);
                assert_eq!(sync.sync_clusters[2].epoch_offset, 1);
                assert_eq!(sync.sync_clusters[2].epoch_queue.peers.len(), 2);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change
    }
}
