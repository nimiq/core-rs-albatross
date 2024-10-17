use std::{
    collections::BTreeSet,
    fmt::{Debug, Formatter},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{FutureExt, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain_interface::Direction;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{Network, NetworkEvent},
    request::RequestError,
};
use nimiq_primitives::{policy::Policy, slots_allocation::Validators};
use nimiq_utils::spawn;
use parking_lot::RwLock;
use thiserror::Error;

use crate::{
    messages::{RequestMissingBlocks, ResponseBlocksError},
    sync::{peer_list::PeerList, sync_queue::SyncQueue},
};

#[derive(Debug)]
pub struct BlockRequestResult<N: Network> {
    pub target_block_number: u32,
    pub target_block_hash: Blake2bHash,
    pub epoch_validators: Validators,
    pub blocks: Vec<Block>,
    pub sender: N::PeerId,
}

#[derive(Clone)]
pub struct MissingBlockRequest {
    /// The block number of the requested block.
    pub target_block_number: u32,
    /// The hash the last block which is requested with this request.
    pub target_block_hash: Blake2bHash,
    /// The validators of the epoch the target block is from.
    pub epoch_validators: Validators,
    /// List of locator hashes of blocks, sorted from newest to oldest.
    pub locators: Vec<Blake2bHash>,
    /// Flag indicating whether bodies must be included or omitted.
    pub include_body: bool,
    /// The Direction the request is to be executed in.
    /// See [RequestMissingBlocks] for details on the effect.
    pub direction: Direction,
}

impl Debug for MissingBlockRequest {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        let mut dbg = f.debug_struct("MissingBlockRequest");
        dbg.field("target_block_number", &self.target_block_number);
        dbg.field("target_block_hash", &self.target_block_hash);
        dbg.field("num_locators", &self.locators.len());
        dbg.field("include_body", &self.include_body);
        dbg.field("direction", &self.direction);
        dbg.finish()
    }
}

#[derive(Debug, Clone)]
pub struct MissingBlockResponse<N: Network> {
    pub target_block_number: u32,
    pub target_block_hash: Blake2bHash,
    pub epoch_validators: Validators,
    pub blocks: Vec<Block>,
    pub sender: N::PeerId,
}

#[derive(Clone, Debug, Error)]
pub enum MissingBlockError {
    #[error("request: {0}")]
    Request(RequestError),
    #[error("response: {0}")]
    Response(ResponseBlocksError),
}

#[derive(Debug, Error)]
#[error("Sync queue error. Target block hash: {target_block_hash}, target block number {target_block_number}")]
pub struct SyncQueueError {
    /// Target block hash
    pub target_block_hash: Blake2bHash,
    /// Target block number
    pub target_block_number: u32,
}

/// Peer Tracking & Block Request Component.
/// We use this component to request missing blocks from peers.
///
/// This component has:
///
/// - The sync queue which manages the requests and responses.
/// - The peers list.
/// - The network stream of events used to remove the peers that have left.  
/// - Whether we include the body of a block.
///
/// The public interface allows to request blocks, which are not immediately returned.
/// The blocks instead are returned by polling the component.
pub struct BlockRequestComponent<N: Network> {
    sync_queue: SyncQueue<N, MissingBlockRequest, MissingBlockResponse<N>, MissingBlockError, ()>, // requesting missing blocks from peers
    peers: Arc<RwLock<PeerList<N>>>,
    include_body: bool,
    /// Pending requests.
    pending_requests: BTreeSet<Blake2bHash>,
}

impl<N: Network> BlockRequestComponent<N> {
    const NUM_PENDING_BLOCKS: usize = 5;

    pub fn new(network: Arc<N>, include_body: bool) -> Self {
        let peers = Arc::new(RwLock::new(PeerList::default()));
        let mut network_event_rx = network.subscribe_events();

        // Poll network events to remove peers.
        let peers_weak = Arc::downgrade(&peers);
        spawn(async move {
            while let Some(result) = network_event_rx.next().await {
                if let Ok(NetworkEvent::PeerLeft(peer_id)) = result {
                    // Remove peers that left.
                    let peers = match peers_weak.upgrade() {
                        Some(peers) => peers,
                        None => break,
                    };

                    trace!(%peer_id, "Removing peer from live sync");
                    peers.write().remove_peer(&peer_id);
                }
            }
        });

        Self {
            sync_queue: SyncQueue::with_verification(
                network,
                vec![],
                Arc::clone(&peers),
                Self::NUM_PENDING_BLOCKS,
                |request, network, peer_id| {
                    async move {
                        match Self::request_missing_blocks_from_peer(
                            network,
                            peer_id,
                            request.target_block_hash.clone(),
                            request.locators,
                            request.include_body,
                            request.direction,
                        )
                        .await
                        {
                            Ok(Ok(missing_blocks)) => Ok(MissingBlockResponse {
                                target_block_number: request.target_block_number,
                                target_block_hash: request.target_block_hash,
                                epoch_validators: request.epoch_validators,
                                blocks: missing_blocks,
                                sender: peer_id,
                            }),
                            Ok(Err(error)) => Err(MissingBlockError::Response(error)),
                            Err(error) => Err(MissingBlockError::Request(error)),
                        }
                    }
                    .boxed()
                },
                |request, response, _| {
                    // We check general consistency for the response:
                    // 1. Check that the blocks end on target block or macro block
                    // 2. Verify macro block signature (last block)
                    // 3. Check that hash chain verifies
                    // We need to pass through the validators once we reach a new epoch
                    // to be able to verify macro blocks

                    let blocks = &mut response.blocks;

                    // Size checks.
                    if blocks.is_empty() {
                        log::debug!("Received empty missing blocks response");
                        return false;
                    }

                    if blocks.len() > Policy::blocks_per_batch() as usize {
                        log::debug!(
                            blocks_len = blocks.len(),
                            "Received missing blocks response that is too large"
                        );
                        return false;
                    }

                    for block in blocks.iter() {
                        if block.body().is_some() != request.include_body {
                            log::error!(
                                is_macro = block.is_macro(),
                                has_body = block.body().is_some(),
                                include_body = request.include_body,
                                "Received block with body where none was expected or vice versa",
                            );
                            return false;
                        }
                    }

                    // Checks that the first block's parent was part of the block locators.
                    let first_block = blocks.first().unwrap(); // cannot be empty
                    if !request.locators.contains(first_block.parent_hash()) {
                        log::error!("Received invalid chain of missing blocks (first block's parent not in block locators)");
                        return false;
                    }

                    if first_block.block_number() > request.target_block_number {
                        log::error!(
                            first_block = first_block.block_number(),
                            request.target_block_number,
                            "Received invalid chain of missing blocks (first block > target)"
                        );
                        return false;
                    }

                    // Check that the last block is valid.
                    // The last block must be the target block or a macro block.
                    let last_block = blocks.last_mut().unwrap(); // cannot be empty
                    let block_hash = last_block.hash_cached();
                    if !(last_block.is_macro()
                        || (last_block.block_number() == request.target_block_number
                            && block_hash == request.target_block_hash))
                    {
                        log::error!(
                            request.target_block_number,
                            %block_hash,
                            %request.target_block_hash,
                            "Received invalid missing blocks (invalid target block)"
                        );
                        return false;
                    }

                    // Check that the hash chain of missing blocks is valid.
                    // Also checks block numbers.
                    for i in 1..blocks.len() {
                        let previous_block_hash = blocks[i - 1].hash_cached();

                        if blocks[i].block_number() != blocks[i - 1].block_number() + 1
                            || blocks[i].block_number() > request.target_block_number
                            || blocks[i].parent_hash() != &previous_block_hash
                        {
                            log::error!("Received invalid chain of missing blocks");
                            return false;
                        }
                    }

                    // If it is a macro block, also check the signatures.
                    let last_block = response.blocks.last().unwrap(); // cannot be empty
                    if last_block.is_macro() {
                        if let Err(e) = last_block.verify_validators(&request.epoch_validators) {
                            log::error!(
                                last_block = last_block.block_number(),
                                error = %e,
                                "Received invalid chain of missing blocks (macro block does not verify)"
                            );
                            return false;
                        }
                    }

                    true
                },
                (),
            ),
            peers,
            include_body,
            pending_requests: BTreeSet::new(),
        }
    }

    async fn request_missing_blocks_from_peer(
        network: Arc<N>,
        peer_id: N::PeerId,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
        include_body: bool,
        direction: Direction,
    ) -> Result<Result<Vec<Block>, ResponseBlocksError>, RequestError> {
        network
            .request::<RequestMissingBlocks>(
                RequestMissingBlocks {
                    locators,
                    target_hash: target_block_hash,
                    include_body,
                    direction,
                },
                peer_id,
            )
            .await
            .map(|response| response.map(|r| r.blocks))
    }

    pub fn request_missing_blocks(
        &mut self,
        target_block_number: u32,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
        direction: Direction,
        epoch_validators: Validators,
        first_peer_id: Option<N::PeerId>,
    ) {
        self.pending_requests.insert(target_block_hash.clone());
        self.sync_queue.add_ids(vec![(
            MissingBlockRequest {
                target_block_number,
                target_block_hash,
                epoch_validators,
                locators,
                direction,
                include_body: self.include_body,
            },
            first_peer_id,
        )]);
    }

    pub fn is_pending(&self, target_block_hash: &Blake2bHash) -> bool {
        self.pending_requests.contains(target_block_hash)
    }

    pub fn has_no_pending_requests(&self) -> bool {
        self.pending_requests.is_empty()
    }

    pub fn add_peer(&self, peer_id: N::PeerId) {
        self.peers.write().add_peer(peer_id);
    }

    pub fn take_peer(&self, peer_id: &N::PeerId) -> Option<N::PeerId> {
        if self.peers.write().remove_peer(peer_id) {
            return Some(*peer_id);
        }
        None
    }

    pub fn num_peers(&self) -> usize {
        self.peers.read().len()
    }

    pub fn peers(&self) -> Vec<N::PeerId> {
        self.peers.read().peers().to_vec()
    }

    pub fn peer_list(&self) -> Arc<RwLock<PeerList<N>>> {
        Arc::clone(&self.peers)
    }
}

impl<N: Network> Stream for BlockRequestComponent<N> {
    type Item = Result<BlockRequestResult<N>, SyncQueueError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Poll self.sync_queue, return results.
        while let Poll::Ready(result) = self.sync_queue.poll_next_unpin(cx) {
            match result {
                Some(Ok(response)) => {
                    self.pending_requests.remove(&response.target_block_hash);
                    return Poll::Ready(Some(Ok(BlockRequestResult {
                        target_block_number: response.target_block_number,
                        target_block_hash: response.target_block_hash,
                        epoch_validators: response.epoch_validators,
                        blocks: response.blocks,
                        sender: response.sender,
                    })));
                }
                Some(Err(request)) => {
                    self.pending_requests.remove(&request.target_block_hash);
                    debug!(
                        request.target_block_number,
                        ?request.target_block_hash,
                        "Failed to retrieve missing blocks"
                    );
                    return Poll::Ready(Some(Err(SyncQueueError {
                        target_block_hash: request.target_block_hash,
                        target_block_number: request.target_block_number,
                    })));
                }
                None => {}
            }
        }

        Poll::Pending
    }
}
