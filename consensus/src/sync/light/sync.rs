use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use futures::{future::BoxFuture, FutureExt};
use nimiq_block::Block;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_genesis::HardcodedElection;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{CloseReason, Network, SubscribeEvents},
    request::RequestError,
};
use nimiq_primitives::policy::Policy;
use nimiq_utils::{spawn, stream::FuturesUnordered};
use parking_lot::RwLock;

#[cfg(feature = "full")]
use crate::messages::{HistoryChunk, HistoryChunkError, RequestHistoryChunk};
use crate::{
    messages::BlockError,
    sync::{
        peer_list::PeerList,
        sync_interface::{EpochIds, MacroSync},
        sync_queue::SyncQueue,
    },
};

#[cfg(feature = "full")]
/// Struct used to track the progress of the validity window chunk process.
pub struct ValidityChunkRequest {
    /// This corresponds to the block that should be used to verify the proof.
    pub verifier_block_number: u32,
    /// The root hash that should be used to verify the proof.
    pub root_hash: Blake2bHash,
    /// The chunk index that was requested.
    pub chunk_index: u32,
    /// Flag to indicate if there is an election block within the validity window
    pub election_in_window: bool,
    /// Number of items in the previous requested chunk (for cases where we adopt a new macro head)
    pub last_chunk_items: Option<usize>,
}

/// Sync queue pending size (max in-flight block / chunk requests per queue).
const PENDING_SIZE: usize = 5;

/// Error returned by the `macro_block_queue` request function. Both variants are **retryable**:
/// `SyncQueue` fails the request over to another peer. A peer returning a block that does not match
/// the requested hash (`BlockError::ResponseHashMismatch`) is the only provably-malicious case, and
/// it is instead surfaced as a successful-but-erroneous output so the consumer can ban that specific
/// peer — never converted into this (retryable) error.
#[derive(Debug)]
pub(crate) enum MacroBlockError {
    /// Transport-level request error (timeout, connection closed, ...).
    Request(RequestError),
    /// Remote block-level error other than a hash mismatch (e.g. the peer does not have the block
    /// it was asked for). The responder is not at fault; retry from another peer.
    Block(BlockError),
}

impl std::fmt::Display for MacroBlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MacroBlockError::Request(error) => write!(f, "request error: {error}"),
            MacroBlockError::Block(error) => write!(f, "block error: {error}"),
        }
    }
}

/// The LightMacroSync is one type of MacroSync and it is essentially a stream,
/// that operates on a per peer basis, emitting peers either as Outdated or Good.
/// To do this, it will:
///   1. Request epoch IDs from the peer
///   2. Request the last (if any) election or checkpoint blocks
///
/// If during the process, a peer is deemed as outdated, then it is emitted
pub struct LightMacroSync<TNetwork: Network> {
    /// The blockchain
    pub(crate) blockchain: BlockchainProxy,
    /// Reference to the network
    pub(crate) network: Arc<TNetwork>,
    /// Stream for peer joined and peer left events
    pub(crate) network_event_rx: SubscribeEvents<TNetwork::PeerId>,
    /// The stream for epoch ids requests
    pub(crate) epoch_ids_stream:
        FuturesUnordered<BoxFuture<'static, Option<EpochIds<TNetwork::PeerId>>>>,

    /// Hardcoded election block to bootstrap from on the pico sync fallback path; `None` for
    /// light/full sync (which sync the election chain from genesis and only verify the checkpoint).
    /// When set and the chain is still behind it, the checkpoint election block is fetched through
    /// `macro_block_queue` and seeded via `apply_macro_block` (see `add_peer`).
    pub(crate) bootstrap_checkpoint: Option<HardcodedElection>,

    /// Bounded, ordered, failover queue for fetching macro (election) block headers: the checkpoint
    /// seed, the forward catch-up after it, and the full-node `previous_slots` recovery block all go
    /// through here. Requests are bounded (`PENDING_SIZE`), responses applied in request (epoch)
    /// order, and a timed-out block is re-requested from another peer instead of stalling. Its peer
    /// pool is maintained via `macro_block_queue.add_peer`/`remove_peer`.
    pub(crate) macro_block_queue: SyncQueue<
        TNetwork,
        Blake2bHash,
        (Blake2bHash, Result<Block, BlockError>, TNetwork::PeerId),
        MacroBlockError,
        (),
    >,

    /// Macro block hashes currently queued or in flight in `macro_block_queue`. Deduplicates
    /// overlapping `epoch_ids` (and seed requests) from multiple peers; entries are removed once the
    /// block is applied or its request is exhausted.
    pub(crate) in_flight_macro_blocks: HashSet<Blake2bHash>,

    /// Peers waiting for `macro_block_queue` to advance our head to their reported target block
    /// number. Once reached they are re-queried for epoch ids (emitting `Good` when nothing is
    /// left, or enqueuing the next epochs if the tip moved). Drives the checkpoint seed and forward
    /// catch-up.
    pub(crate) waiting_macro_peers: HashMap<TNetwork::PeerId, u32>,

    /// Peers to re-query once a specific queued block resolves, keyed by that block's hash. Used for
    /// the full-node `previous_slots` recovery fetch, whose `update_previous_slots` apply does not
    /// advance the head (so `waiting_macro_peers` can't drive it).
    pub(crate) pending_followup_requests: HashMap<Blake2bHash, Vec<TNetwork::PeerId>>,

    #[cfg(feature = "full")]
    /// The validity (history chunks) queue
    pub(crate) validity_queue: SyncQueue<
        TNetwork,
        RequestHistoryChunk,
        (
            RequestHistoryChunk,
            Result<HistoryChunk, HistoryChunkError>,
            TNetwork::PeerId,
        ),
        RequestError,
        (),
    >,

    #[cfg(feature = "full")]
    /// Used to track the validity chunks we are requesting
    pub(crate) validity_requests: Option<ValidityChunkRequest>,
    #[cfg(feature = "full")]
    /// The peers we are currently syncing with
    pub(crate) syncing_peers: HashSet<TNetwork::PeerId>,
    #[cfg(feature = "full")]
    /// A vec of all the peers that we successfully synced with
    pub(crate) synced_validity_peers: Vec<TNetwork::PeerId>,
    #[cfg(feature = "full")]
    /// Minimum distance to light sync in #blocks from the peers head.
    pub(crate) full_sync_threshold: u32,
}

impl<TNetwork: Network> LightMacroSync<TNetwork> {
    pub fn new(
        blockchain: BlockchainProxy,
        network: Arc<TNetwork>,
        network_event_rx: SubscribeEvents<TNetwork::PeerId>,
        full_sync_threshold: u32,
        bootstrap_checkpoint: Option<HardcodedElection>,
    ) -> Self {
        // A hardcoded checkpoint must point at an election block; otherwise the pico bootstrap seed
        // branch (gated on `block.is_election()` in `apply_macro_block`) never fires and the chain
        // would silently fail to seed. Catch a release-time misconfiguration early.
        //
        // NOTE: this only validates the *height*. The `hash` and `block_number` are hand-pasted per
        // release and MUST come from the same block — they cannot be cross-checked here without the
        // block itself. If they disagree, the fetched (by-hash) block won't match the seed condition
        // and the pico bootstrap silently fails to seed (it no longer bans peers — see `apply_macro_block`).
        debug_assert!(
            bootstrap_checkpoint
                .as_ref()
                .is_none_or(|cp| Policy::is_election_block_at(cp.block_number)),
            "bootstrap checkpoint must be an election block"
        );

        // Queue for all macro-block fetches (seed, forward catch-up, previous_slots recovery).
        // Ungated (used by light/pico too). The queue owns its peer pool, maintained via
        // `macro_block_queue.add_peer`/`remove_peer` (same pattern as `validity_queue`).
        let macro_block_queue = SyncQueue::with_verification(
            Arc::clone(&network),
            Vec::<(Blake2bHash, Option<_>)>::new(),
            Arc::new(RwLock::new(PeerList::default())),
            PENDING_SIZE,
            move |block_hash, network, peer_id| {
                async move {
                    match Self::request_macro_block(network, peer_id, block_hash.clone()).await {
                        // Transport failure: retry from another peer.
                        Err(error) => Err(MacroBlockError::Request(error)),
                        // A hash mismatch is provably the responder's fault: surface it (not
                        // retryable) so the consumer bans that specific peer.
                        Ok(Err(error @ BlockError::ResponseHashMismatch)) => {
                            Ok((block_hash, Err(error), peer_id))
                        }
                        // Any other remote block error (e.g. the peer simply does not have the
                        // block it was asked for) is not this peer's fault: retry from another peer
                        // rather than disconnecting an honest failover target.
                        Ok(Err(error)) => Err(MacroBlockError::Block(error)),
                        Ok(Ok(block)) => Ok((block_hash, Ok(block), peer_id)),
                    }
                }
                .boxed()
            },
            |_, _, _| true,
            (),
        );

        #[cfg(feature = "full")]
        let peers = Arc::new(RwLock::new(PeerList::default()));

        #[cfg(feature = "full")]
        let validity_queue = SyncQueue::new(
            Arc::clone(&network),
            Vec::<(RequestHistoryChunk, Option<_>)>::new(),
            peers,
            PENDING_SIZE,
            move |request, network, peer_id| {
                async move {
                    let res = Self::request_validity_window_chunk(
                        network,
                        peer_id,
                        request.epoch_number,
                        request.block_number,
                        request.chunk_index,
                    )
                    .await?;
                    Ok((request, res, peer_id))
                }
                .boxed()
            },
        );

        Self {
            blockchain,
            network,
            network_event_rx,
            epoch_ids_stream: FuturesUnordered::new(),
            #[cfg(feature = "full")]
            full_sync_threshold,
            bootstrap_checkpoint,
            macro_block_queue,
            in_flight_macro_blocks: HashSet::new(),
            waiting_macro_peers: HashMap::new(),
            pending_followup_requests: HashMap::new(),
            #[cfg(feature = "full")]
            validity_requests: None,
            #[cfg(feature = "full")]
            syncing_peers: HashSet::new(),
            #[cfg(feature = "full")]
            validity_queue,
            #[cfg(feature = "full")]
            synced_validity_peers: Vec::new(),
        }
    }

    pub fn remove_peer(&mut self, peer_id: TNetwork::PeerId) {
        // Drop the peer from the macro-block fetch pool so the queue stops requesting/failing over
        // to it, and forget any pending "waiting for our head to reach its target" bookkeeping.
        // `pending_followup_requests` entries are left to drain on block resolution (the re-query is
        // `has_peer`-guarded), so no scrub is needed here.
        self.macro_block_queue.remove_peer(&peer_id);
        self.waiting_macro_peers.remove(&peer_id);
    }

    pub fn disconnect_peer(&mut self, peer_id: TNetwork::PeerId, reason: CloseReason) {
        // Remove all pending peer bookkeeping (if any)
        self.remove_peer(peer_id);
        let network = Arc::clone(&self.network);
        // We disconnect from this peer
        spawn(Box::pin({
            async move {
                network.disconnect_peer(peer_id, reason).await;
            }
        }));
    }

    /// Starts epoch-id macro sync with a peer.
    pub(crate) fn request_epoch_ids_from(&mut self, peer_id: TNetwork::PeerId) {
        let future =
            Self::request_epoch_ids(self.blockchain.clone(), Arc::clone(&self.network), peer_id)
                .boxed();
        self.epoch_ids_stream.push(future);
    }
}

impl<TNetwork: Network> MacroSync<TNetwork::PeerId> for LightMacroSync<TNetwork> {
    const MAX_REQUEST_EPOCHS: u16 = 1000; // TODO: Use other value

    fn add_peer(&mut self, peer_id: TNetwork::PeerId) {
        // Make the peer available to the macro-block fetch queue (idempotent) so it can serve
        // election blocks and act as a failover target.
        self.macro_block_queue.add_peer(peer_id);

        // If we have a hardcoded checkpoint and have not reached it yet (pico sync fallback path),
        // bootstrap from it instead of re-syncing the whole election chain from genesis (infeasible
        // for pico/browser clients): enqueue the checkpoint election block (deduplicated; the
        // request layer verifies the response matches the hash). The queue applies it first via
        // `apply_macro_block`, which seeds the chain; once our head reaches the checkpoint the
        // tip-follow loop re-queries this peer for epoch ids to sync forward.
        if let Some(checkpoint) = &self.bootstrap_checkpoint
            && self.blockchain.read().block_number() < checkpoint.block_number
        {
            let hash = checkpoint.hash.clone();
            let target = checkpoint.block_number;
            // Only request the checkpoint if it isn't already in flight; otherwise this peer just
            // joins the queue's pool as a failover target and waits for our head to reach it.
            if self.in_flight_macro_blocks.insert(hash.clone()) {
                info!(%peer_id, "Requesting checkpoint block from peer");
                self.macro_block_queue.add_ids(vec![(hash, Some(peer_id))]);
            }
            self.waiting_macro_peers.insert(peer_id, target);
            return;
        }

        info!(%peer_id, "Requesting epoch ids from peer");
        self.request_epoch_ids_from(peer_id);
    }

    fn fallback(&mut self, _peers: Vec<TNetwork::PeerId>) {
        unimplemented!()
    }

    fn drain_peers(&mut self) -> Vec<TNetwork::PeerId> {
        unimplemented!()
    }
}
