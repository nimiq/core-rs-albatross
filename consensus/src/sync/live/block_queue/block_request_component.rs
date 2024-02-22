use std::{
    collections::BTreeSet,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{FutureExt, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain_interface::Direction;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{Network, NetworkEvent, SubscribeEvents},
    request::RequestError,
};
use nimiq_primitives::{policy::Policy, slots_allocation::Validators};
use parking_lot::RwLock;
use thiserror::Error;

use crate::{
    messages::{RequestMissingBlocks, ResponseBlocksError},
    sync::{peer_list::PeerList, sync_queue::SyncQueue},
};

#[derive(Debug)]
pub enum BlockRequestComponentEvent {
    /// Received blocks for a target block number and block hash.
    ReceivedBlocks(u32, Blake2bHash, Validators, Vec<Block>),
}

#[derive(Debug, Clone)]
pub struct MissingBlockRequest {
    /// The block number of the requested block.
    pub target_block_number: u32,
    /// The hash the last block which is requested with this request.
    pub target_block_hash: Blake2bHash,
    /// The validators of the epoch the target block is from.
    pub epoch_validators: Validators,
    /// List of locator hashes of blocks, sorted from newest to oldest.
    pub locators: Vec<Blake2bHash>,
    /// Indicator whether or not micro bodies must be included or omitted.
    /// Macro bodies are always included.
    pub include_micro_bodies: bool,
    /// The Direction the request is to be executed in.
    /// See [RequestMissingBlocks] for details on the effect.
    pub direction: Direction,
}

#[derive(Debug, Clone)]
pub struct MissingBlockResponse {
    pub target_block_number: u32,
    pub target_block_hash: Blake2bHash,
    pub epoch_validators: Validators,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Error)]
pub enum MissingBlockError {
    #[error("request: {0}")]
    Request(RequestError),
    #[error("response: {0}")]
    Response(ResponseBlocksError),
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
    sync_queue: SyncQueue<N, MissingBlockRequest, MissingBlockResponse, MissingBlockError, ()>, // requesting missing blocks from peers
    peers: Arc<RwLock<PeerList<N>>>,
    network_event_rx: SubscribeEvents<N::PeerId>,
    include_micro_bodies: bool,
    /// Pending requests.
    pending_requests: BTreeSet<Blake2bHash>,
}

impl<N: Network> BlockRequestComponent<N> {
    const NUM_PENDING_BLOCKS: usize = 5;

    pub fn new(network: Arc<N>, include_micro_bodies: bool) -> Self {
        let peers = Arc::new(RwLock::new(PeerList::default()));
        let network_event_rx = network.subscribe_events();
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
                            request.include_micro_bodies,
                            request.direction,
                        )
                        .await
                        {
                            Ok(Ok(missing_blocks)) => Ok(MissingBlockResponse {
                                target_block_number: request.target_block_number,
                                target_block_hash: request.target_block_hash,
                                epoch_validators: request.epoch_validators,
                                blocks: missing_blocks,
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
                    let blocks = &response.blocks;

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
                    let last_block = blocks.last().unwrap(); // cannot be empty
                    let block_hash = last_block.hash();

                    // The last block must be the target block or a macro block.
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
                    let mut previous = first_block;
                    for block in blocks.iter().skip(1) {
                        if block.block_number() == previous.block_number() + 1
                            && block.block_number() <= request.target_block_number
                            && block.parent_hash() == &previous.hash()
                        {
                            previous = block;
                        } else {
                            log::error!("Received invalid chain of missing blocks");
                            return false;
                        }
                    }

                    // If it is a macro block, also check the signatures.
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
            network_event_rx,
            include_micro_bodies,
            pending_requests: BTreeSet::new(),
        }
    }

    async fn request_missing_blocks_from_peer(
        network: Arc<N>,
        peer_id: N::PeerId,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
        include_micro_bodies: bool,
        direction: Direction,
    ) -> Result<Result<Vec<Block>, ResponseBlocksError>, RequestError> {
        network
            .request::<RequestMissingBlocks>(
                RequestMissingBlocks {
                    locators,
                    target_hash: target_block_hash,
                    include_micro_bodies,
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
        pubsub_id: Option<N::PubsubId>,
    ) {
        self.pending_requests.insert(target_block_hash.clone());
        self.sync_queue.add_ids(vec![(
            MissingBlockRequest {
                target_block_number,
                target_block_hash,
                epoch_validators,
                locators,
                direction,
                include_micro_bodies: self.include_micro_bodies,
            },
            pubsub_id,
        )]);
    }

    pub fn is_pending(&self, target_block_hash: &Blake2bHash) -> bool {
        self.pending_requests.contains(target_block_hash)
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
    type Item = BlockRequestComponentEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Poll network events to remove peers.
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            if let Ok(NetworkEvent::PeerLeft(peer_id)) = result {
                // Remove peers that left.
                self.peers.write().remove_peer(&peer_id);
            }
        }

        // Poll self.sync_queue, return results.
        while let Poll::Ready(Some(result)) = self.sync_queue.poll_next_unpin(cx) {
            match result {
                Ok(response) => {
                    self.pending_requests.remove(&response.target_block_hash);
                    return Poll::Ready(Some(BlockRequestComponentEvent::ReceivedBlocks(
                        response.target_block_number,
                        response.target_block_hash,
                        response.epoch_validators,
                        response.blocks,
                    )));
                }
                Err(request) => {
                    self.pending_requests.remove(&request.target_block_hash);
                    debug!(
                        request.target_block_number,
                        ?request.target_block_hash,
                        "Failed to retrieve missing blocks"
                    );
                    // TODO: Do we need to do anything else?
                    // We might want to delete the target hash from our buffer
                    // since none of our peers is sending us a good response.
                }
            }
        }

        Poll::Pending
    }
}
