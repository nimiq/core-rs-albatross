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
use utils::math::CeilingDiv;

use crate::consensus_agent::ConsensusAgent;
use crate::messages::{BatchSetInfo, BlockHashType, HistoryChunk, RequestBlockHashesFilter};
use crate::sync::sync_queue::SyncQueue;

struct PendingBatchSet {
    block: MacroBlock,
    history_len: usize,
    history: Vec<ExtendedTransaction>,
}
impl PendingBatchSet {
    fn is_complete(&self) -> bool {
        self.history_len == self.history.len()
    }

    fn epoch_number(&self) -> u32 {
        policy::epoch_at(self.block.header.block_number)
    }
}

pub struct BatchSet {
    block: MacroBlock,
    history: Vec<ExtendedTransaction>,
}

struct SyncCluster<TPeer: Peer> {
    ids: Vec<Blake2bHash>,
    epoch_offset: usize,

    batch_set_queue: SyncQueue<TPeer, Blake2bHash, BatchSetInfo>,
    history_queue: SyncQueue<TPeer, (u32, usize), (u32, HistoryChunk)>,

    pending_batch_sets: VecDeque<PendingBatchSet>,

    adopted_batch_set: bool,
    blockchain: Arc<Blockchain>,
}

impl<TPeer: Peer + 'static> SyncCluster<TPeer> {
    const NUM_PENDING_BATCH_SETS: usize = 5;
    const NUM_PENDING_CHUNKS: usize = 12;

    fn new(ids: Vec<Blake2bHash>, epoch_offset: usize, peers: Vec<Weak<ConsensusAgent<TPeer>>>, blockchain: Arc<Blockchain>) -> Self {
        let batch_set_queue = SyncQueue::new(ids.clone(), peers.clone(), Self::NUM_PENDING_BATCH_SETS, |id, peer| {
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
            ids,
            epoch_offset,
            batch_set_queue,
            history_queue,
            pending_batch_sets: VecDeque::with_capacity(Self::NUM_PENDING_BATCH_SETS),
            adopted_batch_set: false,
            blockchain,
        }
    }

    fn on_epoch_received(&mut self, epoch: BatchSetInfo) -> Result<(), SyncClusterResult> {
        // TODO Verify macro blocks and their ordering
        // Currently we only do a very basic check here
        let current_block_number = self.blockchain.block_number();
        if epoch.block.header.block_number < current_block_number {
            debug!("Received outdated epoch at block {}", current_block_number);
            return Err(SyncClusterResult::Outdated);
        }

        // Prepare pending info.
        let mut pending_batch_set = PendingBatchSet {
            block: epoch.block,
            history_len: epoch.history_len as usize,
            history: Vec::new(),
        };

        // If the block is in the same epoch, add already known history.
        let epoch_number = policy::epoch_at(pending_batch_set.block.header.block_number);
        let mut start_index = 0;
        if policy::epoch_at(current_block_number) == epoch_number {
            let num_known = self.blockchain.get_num_extended_transactions(epoch_number, None);
            let num_full_chunks = num_known / history_store::CHUNK_SIZE;
            start_index = num_full_chunks;
            // TODO: Can probably be done more efficiently.
            let known_chunk = self
                .blockchain
                .get_chunk(epoch_number, num_full_chunks * history_store::CHUNK_SIZE, 0, None)
                .expect("History chunk missing");
            pending_batch_set.history = known_chunk.history;
        }

        // Queue history chunks for the given epoch for download.
        let history_chunk_ids = (start_index..((epoch.history_len as usize).ceiling_div(history_store::CHUNK_SIZE)))
            .map(|i| (epoch_number, i))
            .collect();
        debug!("Requesting history for ids: {:?}", history_chunk_ids);
        self.history_queue.add_ids(history_chunk_ids);

        // We keep the epoch in pending_epochs while the history is downloading.
        self.pending_batch_sets.push_back(pending_batch_set);

        Ok(())
    }

    fn on_history_chunk_received(&mut self, epoch_number: u32, history_chunk: HistoryChunk) -> Result<(), SyncClusterResult> {
        // Find epoch in pending_epochs.
        let first_epoch_number = self.pending_batch_sets[0].epoch_number();
        let epoch_index = (epoch_number - first_epoch_number) as usize;
        let epoch = &mut self.pending_batch_sets[epoch_index];

        // TODO: This assumes that we have already filtered responses with no chunk.
        // Verify chunk.
        let chunk = history_chunk.chunk.expect("History chunk missing");
        if !chunk
            .verify(epoch.block.body.as_ref().expect("Missing body").history_root.clone(), epoch.history.len())
            .unwrap_or(false)
        {
            return Err(SyncClusterResult::Error);
        }
        // Add the received history chunk to the pending epoch.
        let mut chunk = chunk.history;
        epoch.history.append(&mut chunk);

        Ok(())
    }

    fn add_peer(&mut self, peer: Weak<ConsensusAgent<TPeer>>) -> bool {
        // TODO keep only one list of peers
        if !self.batch_set_queue.has_peer(&peer) {
            self.batch_set_queue.add_peer(Weak::clone(&peer));
            self.history_queue.add_peer(peer);
            return true;
        }
        false
    }

    fn has_peer(&self, peer: &Weak<ConsensusAgent<TPeer>>) -> bool {
        self.batch_set_queue.has_peer(peer)
    }

    fn peers(&self) -> &Vec<Weak<ConsensusAgent<TPeer>>> {
        &self.batch_set_queue.peers
    }

    fn split_off(&mut self, at: usize) -> Self {
        let ids = self.ids.split_off(at);
        let offset = self.epoch_offset + at;

        // Remove the split-off ids from our epoch queue.
        self.batch_set_queue.truncate_ids(at);

        Self::new(ids, offset, self.batch_set_queue.peers.clone(), Arc::clone(&self.blockchain))
    }

    fn remove_front(&mut self, at: usize) {
        let mut new_cluster = self.split_off(at);
        new_cluster.adopted_batch_set = self.adopted_batch_set;
        *self = new_cluster;
    }
}

impl<TPeer: Peer + 'static> Stream for SyncCluster<TPeer> {
    type Item = Result<BatchSet, SyncClusterResult>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.pending_batch_sets.len() < Self::NUM_PENDING_BATCH_SETS {
            while let Poll::Ready(Some(result)) = self.batch_set_queue.poll_next_unpin(cx) {
                match result {
                    Ok(epoch) => {
                        if let Err(e) = self.on_epoch_received(epoch) {
                            return Poll::Ready(Some(Err(e)));
                        }
                    }
                    Err(_e) => {
                        return Poll::Ready(Some(Err(SyncClusterResult::Error)));
                    } // TODO Error
                }
            }
        }

        while let Poll::Ready(Some(result)) = self.history_queue.poll_next_unpin(cx) {
            match result {
                Ok((epoch_number, history_chunk)) => {
                    if let Err(e) = self.on_history_chunk_received(epoch_number, history_chunk) {
                        return Poll::Ready(Some(Err(e)));
                    }

                    // Emit finished epochs.
                    if self.pending_batch_sets[0].is_complete() {
                        let epoch = self.pending_batch_sets.pop_front().unwrap();
                        let epoch = BatchSet {
                            block: epoch.block,
                            history: epoch.history,
                        };
                        return Poll::Ready(Some(Ok(epoch)));
                    }
                }
                Err(_e) => {
                    return Poll::Ready(Some(Err(SyncClusterResult::Error)));
                } // TODO Error
            }
        }

        // We're done if there are no more epochs to process.
        if self.batch_set_queue.is_empty() && self.pending_batch_sets.is_empty() {
            return Poll::Ready(None);
        }

        Poll::Pending
    }
}

impl<TPeer: Peer> PartialEq for SyncCluster<TPeer> {
    fn eq(&self, other: &Self) -> bool {
        self.epoch_offset == other.epoch_offset && self.batch_set_queue.num_peers() == other.batch_set_queue.num_peers() && self.ids == other.ids
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
            .then_with(|| other.batch_set_queue.num_peers().cmp(&self.batch_set_queue.num_peers())) // Higher peer count first
            .then_with(|| other.ids.len().cmp(&self.ids.len())) // More ids first
            .then_with(|| self.ids.cmp(&other.ids)) //
            .reverse() // We want the best cluster to be *last*
    }
}

struct EpochIds<TPeer: Peer> {
    ids: Vec<Blake2bHash>,
    checkpoint_id: Option<Blake2bHash>, // The most recent checkpoint block in the latest epoch.
    offset: usize,
    sender: Arc<ConsensusAgent<TPeer>>,
}

impl<TPeer: Peer> Clone for EpochIds<TPeer> {
    fn clone(&self) -> Self {
        EpochIds {
            ids: self.ids.clone(),
            checkpoint_id: self.checkpoint_id.clone(),
            offset: self.offset,
            sender: Arc::clone(&self.sender),
        }
    }
}

impl<TPeer: Peer> EpochIds<TPeer> {
    fn get_checkpoint_epoch(&self) -> usize {
        self.offset + self.ids.len()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum SyncClusterResult {
    EpochSuccessful,
    NoMoreEpochs,
    Error,
    Outdated,
}

impl<T, E> From<Result<T, E>> for SyncClusterResult {
    fn from(res: Result<T, E>) -> Self {
        match res {
            Ok(_) => SyncClusterResult::EpochSuccessful,
            Err(_) => SyncClusterResult::Error,
        }
    }
}

pub struct HistorySync<TNetwork: Network> {
    blockchain: Arc<Blockchain>,
    network_event_rx: broadcast::Receiver<NetworkEvent<TNetwork::PeerType>>,
    epoch_ids_stream: FuturesUnordered<BoxFuture<'static, Option<EpochIds<TNetwork::PeerType>>>>,
    epoch_sync_clusters: Vec<SyncCluster<TNetwork::PeerType>>,
    checkpoint_sync_clusters: Vec<SyncCluster<TNetwork::PeerType>>,
    agents: HashMap<Arc<TNetwork::PeerType>, (Arc<ConsensusAgent<TNetwork::PeerType>>, usize)>,
}

impl<TNetwork: Network> HistorySync<TNetwork> {
    const MAX_CLUSTERS: usize = 100;

    pub fn new(blockchain: Arc<Blockchain>, network_event_rx: broadcast::Receiver<NetworkEvent<TNetwork::PeerType>>) -> Self {
        Self {
            blockchain,
            network_event_rx,
            epoch_ids_stream: FuturesUnordered::new(),
            epoch_sync_clusters: Vec::new(),
            checkpoint_sync_clusters: Vec::new(),
            agents: HashMap::new(),
        }
    }

    pub fn agents(&self) -> impl Iterator<Item = &Arc<ConsensusAgent<TNetwork::PeerType>>> {
        self.agents.values().map(|(agent, _)| agent)
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
                RequestBlockHashesFilter::ElectionAndLatestCheckpoint,
            )
            .await;

        match result {
            Ok(block_hashes) => {
                // Get checkpoint id if exists.
                let checkpoint_id = block_hashes
                    .hashes
                    .last()
                    .and_then(|(ty, id)| if *ty == BlockHashType::Checkpoint { Some(id.clone()) } else { None });
                // Filter checkpoint from block hashes and map to hash.
                let epoch_ids = block_hashes
                    .hashes
                    .into_iter()
                    .filter_map(|(ty, id)| if ty == BlockHashType::Election { Some(id) } else { None })
                    .collect();
                Some(EpochIds {
                    ids: epoch_ids,
                    checkpoint_id,
                    offset: epoch_number as usize + 1,
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
        let checkpoint_epoch_offset = epoch_ids.get_checkpoint_epoch();
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

        for cluster in &mut self.epoch_sync_clusters {
            // Check if given epoch_ids and the current cluster potentially overlap.
            if cluster.epoch_offset <= epoch_ids.offset && cluster.epoch_offset + cluster.ids.len() > epoch_ids.offset {
                // Compare epoch ids in the overlapping region.
                let start_offset = epoch_ids.offset - cluster.epoch_offset;
                let len = usize::min(cluster.ids.len() - start_offset, epoch_ids.ids.len() - id_index);
                let match_until = cluster.ids[start_offset..start_offset + len]
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
                    if match_until < cluster.ids.len() - start_offset {
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
                Arc::clone(&self.blockchain),
            ));
            // We do not increment the num_clusters here, as this is done in the loop later on.
        }

        // Now cluster the checkpoint id if present.
        if let Some(checkpoint_id) = epoch_ids.checkpoint_id {
            let mut found_cluster = false;
            for cluster in &mut self.checkpoint_sync_clusters {
                // Currently, we do not need to remove old checkpoint ids from the same peer.
                // Since we only request new epoch ids (and checkpoints) once a peer has 0 clusters,
                // we can never receive an updated checkpoint.
                // When this invariant changes, we need to remove old checkpoints of that peer here!

                // Look for clusters at the same offset with the same hash.
                if cluster.epoch_offset == checkpoint_epoch_offset && cluster.ids[0] == checkpoint_id {
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
                    checkpoint_epoch_offset,
                    vec![Arc::downgrade(&agent)],
                    Arc::clone(&self.blockchain),
                );
                self.checkpoint_sync_clusters.push(cluster);
                self.checkpoint_sync_clusters.sort();
                num_clusters += 1;
            }
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
        self.epoch_sync_clusters.append(&mut new_clusters);
        self.epoch_sync_clusters.sort();
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
        trace!("Pulling epoch ids");
        loop {
            if self.epoch_sync_clusters.len() >= Self::MAX_CLUSTERS {
                // TODO: We still want to get the wakes for the epoch_ids_stream
                //  even if we don't poll it now.
                break;
            }

            if let Poll::Ready(Some(epoch_ids)) = self.epoch_ids_stream.poll_next_unpin(cx) {
                if let Some(epoch_ids) = epoch_ids {
                    // The peer might have disconnected during the request.
                    // FIXME Check if the peer is still connected

                    // FIXME We want to distinguish between "locator hash not found" (i.e. peer is
                    //  on a different chain) and "no more hashes past locator" (we are in sync with
                    //  the peer).
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

        trace!("Syncing epoch clusters ({} clusters)", self.epoch_sync_clusters.len());
        // Poll the best epoch cluster.
        // The best cluster is the last element in sync_clusters, so removing it is cheap.
        while !self.epoch_sync_clusters.is_empty() {
            let best_cluster = self.epoch_sync_clusters.last_mut().expect("sync_clusters no empty");

            let result = match ready!(best_cluster.poll_next_unpin(cx)) {
                Some(Ok(epoch)) => SyncClusterResult::from(self.blockchain.push_history_sync(Block::Macro(epoch.block), &epoch.history)),
                Some(Err(_)) => SyncClusterResult::Error,
                None => SyncClusterResult::NoMoreEpochs,
            };

            trace!("Pushed epoch, result: {:?}", result);

            // If the epoch was successful, the cluster is not done yet
            // and we update the remaining clusters.
            if result == SyncClusterResult::EpochSuccessful {
                let best_cluster = self.epoch_sync_clusters.last_mut().expect("sync_clusters no empty");
                best_cluster.adopted_batch_set = true;

                // Cut off the ids we have already adopted from the start of the next cluster.
                // Empty clusters will be dealt with automatically.
                let current_offset = policy::epoch_at(self.blockchain.election_head().header.block_number) as usize;

                for cluster in self.epoch_sync_clusters.iter_mut().rev() {
                    // If `epoch_ids` includes known blocks, truncate (or discard on fork prior to our accepted state).
                    if cluster.epoch_offset <= current_offset {
                        // TODO: Do we want to remove forks entirely here?
                        cluster.remove_front(current_offset - cluster.epoch_offset + 1);
                    } else {
                        // Due to the sorting, we can break here.
                        break;
                    }
                }
                self.epoch_sync_clusters.sort();
            } else {
                // Evict current best cluster and move to next one.
                let cluster = self.epoch_sync_clusters.pop().expect("sync_clusters not empty");

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

                            if result == SyncClusterResult::NoMoreEpochs && cluster.adopted_batch_set {
                                let future = Self::request_epoch_ids(Arc::clone(&self.blockchain), agent).boxed();
                                self.epoch_ids_stream.push(future);
                            } else {
                                // FIXME: Disconnect peer
                                // agent.peer.close()
                            }
                        }
                    }
                }
            }
        }

        trace!("Syncing checkpoint clusters ({} clusters)", self.checkpoint_sync_clusters.len());
        // When no more epochs are to be processed, we continue with checkpoint blocks.
        // Poll the best checkpoint cluster.
        let current_offset = policy::epoch_at(self.blockchain.election_head().header.block_number) as usize;
        while !self.checkpoint_sync_clusters.is_empty() {
            let best_cluster = self.checkpoint_sync_clusters.last_mut().expect("sync_clusters no empty");

            let result;
            if best_cluster.epoch_offset <= current_offset {
                result = SyncClusterResult::NoMoreEpochs;
            } else {
                result = match ready!(best_cluster.poll_next_unpin(cx)) {
                    Some(Ok(batch)) => SyncClusterResult::from(self.blockchain.push_history_sync(Block::Macro(batch.block), &batch.history)),
                    Some(Err(e)) => e,
                    None => SyncClusterResult::NoMoreEpochs,
                };

                trace!("Pushed checkpoint, result: {:?}", result);
            }

            // Since clusters here are always of length 1, we can remove them immediately.
            // Evict current best cluster and move to next one.
            let cluster = self.checkpoint_sync_clusters.pop().expect("sync_clusters not empty");

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

                        if result != SyncClusterResult::Error {
                            let future = Self::request_epoch_ids(Arc::clone(&self.blockchain), agent).boxed();
                            self.epoch_ids_stream.push(future);
                        } else {
                            // FIXME: Disconnect peer
                            // agent.peer.close()
                        }
                    }
                }
            }
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use nimiq_database::volatile::VolatileEnvironment;
    use nimiq_genesis::NetworkId;
    use nimiq_network_mock::{MockHub, MockNetwork, MockPeer};

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
                checkpoint_id: None,
                offset,
                sender: Arc::clone(&agent),
            }
        }

        let env1 = VolatileEnvironment::new(10).unwrap();
        let blockchain = Arc::new(Blockchain::new(env1, NetworkId::UnitAlbatross).unwrap());

        let mut hub = MockHub::default();

        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let net3 = Arc::new(hub.new_network());
        net1.dial_mock(&net2);
        net1.dial_mock(&net3);
        let peers = net1.get_peers();
        let consensus_agents: Vec<_> = peers.into_iter().map(ConsensusAgent::new).map(Arc::new).collect();

        fn run_test<F>(
            blockchain: &Arc<Blockchain>,
            net: &Arc<MockNetwork>,
            epoch_ids1: EpochIds<MockPeer>,
            epoch_ids2: EpochIds<MockPeer>,
            test: F,
            symmetric: bool,
        ) where
            F: Fn(HistorySync<MockNetwork>),
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
                assert_eq!(sync.epoch_sync_clusters.len(), 2);
                assert_eq!(sync.epoch_sync_clusters[0].ids.len(), 10);
                assert_eq!(sync.epoch_sync_clusters[1].ids.len(), 10);
                assert_eq!(sync.epoch_sync_clusters[0].epoch_offset, 1);
                assert_eq!(sync.epoch_sync_clusters[1].epoch_offset, 1);
                assert_eq!(sync.epoch_sync_clusters[0].batch_set_queue.peers.len(), 1);
                assert_eq!(sync.epoch_sync_clusters[1].batch_set_queue.peers.len(), 1);
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
                assert_eq!(sync.epoch_sync_clusters.len(), 2);
                assert_eq!(sync.epoch_sync_clusters[0].ids.len(), 2);
                assert_eq!(sync.epoch_sync_clusters[0].epoch_offset, 9);
                assert_eq!(sync.epoch_sync_clusters[0].batch_set_queue.peers.len(), 1);
                assert_eq!(sync.epoch_sync_clusters[1].ids.len(), 8);
                assert_eq!(sync.epoch_sync_clusters[1].epoch_offset, 1);
                assert_eq!(sync.epoch_sync_clusters[1].batch_set_queue.peers.len(), 2);
            },
            true,
        );

        // 3) different offset, same history, but second is longer
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 3, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_sync_clusters.len(), 2);
                assert_eq!(sync.epoch_sync_clusters[0].ids.len(), 2);
                assert_eq!(sync.epoch_sync_clusters[0].epoch_offset, 11);
                assert_eq!(sync.epoch_sync_clusters[0].batch_set_queue.peers.len(), 1);
                assert_eq!(sync.epoch_sync_clusters[1].ids.len(), 10);
                assert_eq!(sync.epoch_sync_clusters[1].epoch_offset, 1);
                assert_eq!(sync.epoch_sync_clusters[1].batch_set_queue.peers.len(), 2);
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
                assert_eq!(sync.epoch_sync_clusters.len(), 0);
            },
            true,
        );

        // 5) different offset, same history, but second is shorter
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 5, 3, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_sync_clusters.len(), 2);
                assert_eq!(sync.epoch_sync_clusters[0].ids.len(), 3);
                assert_eq!(sync.epoch_sync_clusters[0].epoch_offset, 8);
                assert_eq!(sync.epoch_sync_clusters[0].batch_set_queue.peers.len(), 1);
                assert_eq!(sync.epoch_sync_clusters[1].ids.len(), 7);
                assert_eq!(sync.epoch_sync_clusters[1].epoch_offset, 1);
                assert_eq!(sync.epoch_sync_clusters[1].batch_set_queue.peers.len(), 2);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change

        // 6) different offset, diverging history, second longer
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 8, 4, Some(6));
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_sync_clusters.len(), 3);
                assert_eq!(sync.epoch_sync_clusters[0].ids.len(), 1);
                assert_eq!(sync.epoch_sync_clusters[0].epoch_offset, 10);
                assert_eq!(sync.epoch_sync_clusters[0].batch_set_queue.peers.len(), 1);
                assert_eq!(sync.epoch_sync_clusters[1].ids.len(), 2);
                assert_eq!(sync.epoch_sync_clusters[1].epoch_offset, 10);
                assert_eq!(sync.epoch_sync_clusters[1].batch_set_queue.peers.len(), 1);
                assert_eq!(sync.epoch_sync_clusters[2].ids.len(), 9);
                assert_eq!(sync.epoch_sync_clusters[2].epoch_offset, 1);
                assert_eq!(sync.epoch_sync_clusters[2].batch_set_queue.peers.len(), 2);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change
    }
}
