use std::collections::VecDeque;

use futures::Stream;
use nimiq_block::Block;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;

use super::live::block_queue::BlockSource;
use crate::{consensus::ResolveBlockRequest, messages::Checkpoint};

/// Trait that defines how a node synchronizes macro blocks
/// The expected functionality is that there could be different methods of syncing but they
/// all must synchronize to the latest macro block. An implementation of this trait requests,
/// process and validates these macro blocks.
/// The quantity of macro blocks needed or extra metadata associated are defined by each
/// implementor of these trait.
pub trait MacroSync<TPeerId>: Stream<Item = MacroSyncReturn<TPeerId>> + Unpin + Send {
    /// The maximum amount of epochs we request for MacroSync
    const MAX_REQUEST_EPOCHS: u16;
    /// Adds a peer to synchronize macro blocks
    fn add_peer(&mut self, peer_id: TPeerId);
    /// Gets the list of peers that are being synced with and removes them from the sync process
    fn collect_peers(&mut self) -> Vec<TPeerId>;
    /// Fallbacks to other macro syncing mechanism.
    /// Currently we can only fallback from Pico to Light macro sync.
    fn fallback(&mut self, peers: Vec<TPeerId>);
}

/// Trait that defines how a node synchronizes receiving the blocks the peers are currently
/// processing.
/// The expected functionality is that there could be different methods of syncing but they
/// all must synchronize to the latest macro block. Once that happened, an implementation of
/// this trait comes into play to start receiving or processing the blocks that the peers are
/// currently processing and/or synchronize micro blocks.
pub trait LiveSync<N: Network>: Stream<Item = LiveSyncEvent<N::PeerId>> + Unpin + Send {
    /// This function will be called each time a Block is received or announced by any of the
    /// peers added into the LiveSync.
    fn push_block(&mut self, block: Block, block_source: BlockSource<N>);
    /// Adds a peer to receive or request blocks from it.
    fn add_peer(&mut self, peer_id: N::PeerId);
    /// Returns the number of peers that are being synced with
    fn num_peers(&self) -> usize;
    /// Returns the list of peers that are being synced with
    fn peers(&self) -> Vec<N::PeerId>;
    /// Returns whether the state sync has finished (or `true` if there is no state sync required)
    fn state_complete(&self) -> bool {
        true
    }
    /// Initiates an attempt to resolve a ResolveBlockRequest.
    fn resolve_block(&mut self, request: ResolveBlockRequest<N>);
    /// The maximum number of blocks a peer can be ahead before it is considered out-of-sync.
    fn acceptance_window_size(&self) -> u32;
}

#[derive(Debug, PartialEq, Eq)]
/// Return type for a `MacroSync`
pub enum MacroSyncReturn<T> {
    /// We have synced to this peer's macro state.
    Good(T),
    /// The peer is behind our own state.
    Outdated(T),
    /// We can't sync with this peer.
    Incompatible(T),
    /// Conflicting peer, can only be returned from the Pico Macro sync
    Conflicting(T),
}

#[derive(Clone, Debug)]
/// Enumeration for events emitted by the Live Sync stream
pub enum LiveSyncEvent<TPeerId> {
    /// Events related to received/accepted or rejected blocks
    PushEvent(LiveSyncPushEvent),
    /// Events related to peer qualifications in the sync
    PeerEvent(LiveSyncPeerEvent<TPeerId>),
}

#[derive(Clone, Debug)]
/// Enumeration for the LiveSync stream events related to blocks
pub enum LiveSyncPushEvent {
    /// An announced block has been accepted
    AcceptedAnnouncedBlock(Blake2bHash),
    /// A buffered block has been accepted
    AcceptedBufferedBlock(Blake2bHash, usize),
    /// Missing blocks were received. The vec of all adopted blocks hashes is given here.
    ReceivedMissingBlocks(Vec<Blake2bHash>),
    /// Block was rejected
    /// (this is only returned in *some* cases blocks were rejected)
    RejectedBlock(Blake2bHash),
    /// Chunks have been accepted for the head block
    /// (note that other accepted chunks won't be announced)
    AcceptedChunks(Blake2bHash),
}

#[derive(Clone, Debug)]
/// Enumeration for the LiveSync stream events related to peers
pub enum LiveSyncPeerEvent<TPeerId> {
    /// Peer is in the past (outdated)
    Behind(TPeerId),
    /// Peer is in the future (advanced)
    Ahead(TPeerId),
}

#[derive(Clone)]
/// This struct is used to request Epochs IDs (hashes) from other peers
/// in order to determine their macro chain state relative to us
pub struct EpochIds<T> {
    /// Indicates if the latest epoch id that was queried was found in the peer's chain
    pub locator_found: bool,
    /// The most recent epoch ids (hashes)
    pub ids: Vec<Blake2bHash>,
    /// The most recent checkpoint block in the latest epoch (if any)
    pub checkpoint: Option<Checkpoint>,
    /// Epoch number corresponding to the first hash in ids
    pub first_epoch_number: usize,
    /// The sender that created this struct
    pub sender: T,
}

impl<T> EpochIds<T> {
    #[inline]
    pub(crate) fn checkpoint_epoch_number(&self) -> usize {
        self.first_epoch_number + self.ids.len()
    }

    #[inline]
    pub(crate) fn last_epoch_number(&self) -> usize {
        self.checkpoint_epoch_number().saturating_sub(1)
    }
}

/// This struct is used to track all the macro requests sent to a particular peer
pub struct PeerMacroRequests {
    /// Number of requests that have been fulfilled
    completed_requests: usize,
    /// A Queue used to track the requests that have been sent, and their respective result
    queued_requests: VecDeque<(Blake2bHash, Option<Block>)>,
}

impl Default for PeerMacroRequests {
    fn default() -> Self {
        Self::new()
    }
}
impl PeerMacroRequests {
    pub fn new() -> Self {
        Self {
            completed_requests: 0,
            queued_requests: VecDeque::new(),
        }
    }

    // Pushes a new request into the queue
    pub fn push_request(&mut self, block_hash: Blake2bHash) {
        self.queued_requests.push_back((block_hash, None))
    }

    // Pops a request from the queue
    pub fn pop_request(&mut self) -> Option<(Blake2bHash, Option<Block>)> {
        self.queued_requests.pop_front()
    }

    // Returns true if the request was updated, false in case the request was not found
    pub fn update_request(&mut self, mut block: Block) -> bool {
        let position = self
            .queued_requests
            .iter()
            .position(|(hash, _)| *hash == block.hash_cached());

        if let Some(position) = position {
            if self.queued_requests[position].1.is_none() {
                // A fulfilled request is only count once
                self.completed_requests += 1;
            }
            // We update our block request.
            // Note: If we receive a response more than once, we use the latest
            let block_hash = block.hash_cached();
            log::trace!(%block_hash, "Updating block request");
            self.queued_requests[position] = (block_hash, Some(block));

            true
        } else {
            log::trace!("Received a response for a block that we didn't expect");
            false
        }
    }

    // Returns true if all the requests have been completed
    pub fn is_ready(&self) -> bool {
        self.queued_requests.len() == self.completed_requests
    }
}
