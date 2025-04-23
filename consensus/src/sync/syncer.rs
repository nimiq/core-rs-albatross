//! Syncer component coordinating macro and live synchronization modes.

use std::{
    collections::HashSet,
    mem,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::{future::BoxFuture, FutureExt, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_network_interface::network::{CloseReason, Network, NetworkEvent, SubscribeEvents};
use nimiq_primitives::policy::Policy;
use nimiq_time::{interval, Interval};
use nimiq_utils::stream::FuturesUnordered;

use super::sync_interface::{
    LiveSync, LiveSyncEvent, LiveSyncPeerEvent, LiveSyncPushEvent, MacroSync, MacroSyncReturn,
};
use crate::{
    consensus::ResolveBlockRequest, messages::RequestHead, sync::live::block_queue::BlockSource,
};

/// Syncer is the main synchronization object inside `Consensus`
/// It has a reference to the main blockchain and network and has two dynamic
/// trait objects:
/// - Macro sync: Synchronizes up to the last macro block
/// - Live Sync: Synchronizes the blockchain to the blocks being processed/announced
///   by the peers.
///
/// These two dynamic trait objects are necessary to implement the different types of
/// synchronization such as: history sync, full sync and light sync.
/// The Syncer handles the interactions between these trait objects, the blockchain and
/// the network.
pub struct Syncer<N: Network, M: MacroSync<N::PeerId>, L: LiveSync<N>> {
    /// Synchronizes the blockchain to the blocks being processed/announced by the peers
    pub live_sync: L,

    /// Synchronizes the blockchain to the latest macro blocks of the peers
    pub macro_sync: M,

    /// A proxy to the blockchain
    blockchain: BlockchainProxy,

    /// A reference to the network
    network: Arc<N>,

    /// A network event subscription
    network_events: SubscribeEvents<N::PeerId>,

    /// The set of peers that macro sync deemed outdated
    outdated_peers: HashSet<N::PeerId>,

    /// The set of peers that macro sync deemed incompatible
    incompatible_peers: HashSet<N::PeerId>,

    /// Interval to regularly check outdated/incompatible peers if they are up-to-date.
    check_interval: Interval,

    /// The ongoing up-to-date checks for incompatible peers.
    pending_checks: FuturesUnordered<BoxFuture<'static, (N::PeerId, bool)>>,

    /// The number of blockchain extensions triggered by block announcements
    accepted_announcements: usize,
}

impl<N: Network, M: MacroSync<N::PeerId>, L: LiveSync<N>> Syncer<N, M, L> {
    const CHECK_INTERVAL: Duration = Duration::from_secs(60);

    /// Creates a new `Syncer` instance with the given blockchain proxy, network,
    /// live sync component, and macro sync component.
    ///
    /// Also subscribes to network events and initializes internal state used
    /// for peer tracking and periodic checks.
    pub fn new(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        live_sync: L,
        macro_sync: M,
    ) -> Syncer<N, M, L> {
        let network_events = network.subscribe_events();
        Syncer {
            live_sync,
            macro_sync,
            blockchain,
            network,
            network_events,
            outdated_peers: Default::default(),
            incompatible_peers: Default::default(),
            check_interval: interval(Self::CHECK_INTERVAL),
            pending_checks: Default::default(),
            accepted_announcements: 0,
        }
    }

    /// Pushes a block to the live sync component.
    pub fn push_block(&mut self, block: Block, block_source: BlockSource<N>) {
        self.live_sync.push_block(block, block_source);
    }

    // Moves a peer into macro sync
    fn move_peer_into_macro_sync(&mut self, peer_id: N::PeerId) {
        debug!(%peer_id, "Adding peer to macro sync");
        self.macro_sync.add_peer(peer_id);
    }

    /// Moves a peer into live sync
    pub fn move_peer_into_live_sync(&mut self, peer_id: N::PeerId) {
        debug!(%peer_id, "Adding peer to live sync");
        self.live_sync.add_peer(peer_id);
    }

    /// Returns the number of peers currently participating in live sync.
    pub fn num_peers(&self) -> usize {
        self.live_sync.num_peers()
    }

    /// Returns the list of peer IDs currently in live sync.
    pub fn peers(&self) -> Vec<N::PeerId> {
        self.live_sync.peers()
    }

    /// Returns the number of accepted block announcements.
    pub fn accepted_block_announcements(&self) -> usize {
        self.accepted_announcements
    }

    /// Indicates whether the live sync state has been fully synchronized.
    pub fn state_complete(&self) -> bool {
        self.live_sync.state_complete()
    }

    /// Initiates an attempt to resolve a ResolveBlockRequest.
    pub fn resolve_block(&mut self, request: ResolveBlockRequest<N>) {
        self.live_sync.resolve_block(request)
    }

    // Moves all peers marked as outdated into macro sync.
    fn check_outdated_peers(&mut self) {
        for peer_id in mem::take(&mut self.outdated_peers) {
            self.move_peer_into_macro_sync(peer_id);
        }
    }

    // Triggers a compatibility check for all peers marked as incompatible.
    fn check_incompatible_peers(&mut self) {
        for peer_id in mem::take(&mut self.incompatible_peers) {
            self.check_incompatible_peer(peer_id);
        }
    }

    // Initiates an asynchronous check to determine if a previously incompatible peer
    // has caught up and is now in sync within the `acceptance_window_size`.
    fn check_incompatible_peer(&mut self, peer_id: N::PeerId) {
        let blockchain = self.blockchain.clone();
        let network = Arc::clone(&self.network);
        let acceptance_window_size = self.live_sync.acceptance_window_size();

        debug!(%peer_id, "Checking if incompatible peer is in sync");

        let future = async move {
            let synced =
                Self::is_peer_synced(blockchain, network, peer_id, acceptance_window_size).await;
            (peer_id, synced)
        }
        .boxed();

        self.pending_checks.push(future);
    }

    /// Checks whether the given peer is considered in sync with the local blockchain state.
    async fn is_peer_synced(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        peer_id: N::PeerId,
        acceptance_window_size: u32,
    ) -> bool {
        // Request the peer's head state.
        // Disconnect the peer if the request fails.
        let head = match network.request(RequestHead {}, peer_id).await {
            Ok(head) => head,
            Err(e) => {
                debug!(%peer_id, ?e, "Head request to incompatible peer failed");
                network.disconnect_peer(peer_id, CloseReason::Other).await;
                return false;
            }
        };

        // Check the peer's head block number. We consider the peer synced if the peer is
        // at most one batch behind us.
        let peer_block_number = head.block_number;
        let peer_batch = Policy::batch_at(peer_block_number);
        let our_block_number = blockchain.read().block_number();
        let our_batch = Policy::batch_at(our_block_number);

        if peer_batch < our_batch.saturating_sub(1) {
            debug!(
                %peer_id,
                peer_batch,
                our_batch,
                "Incompatible peer is in a different batch"
            );
            return false;
        }

        // Check that the peer is not too far ahead.
        if peer_block_number > our_block_number + acceptance_window_size {
            debug!(
                %peer_id,
                peer_block_number,
                our_block_number,
                acceptance_window_size,
                "Incompatible peer is too far ahead"
            );
            return false;
        }

        true
    }

    fn remove_peer(&mut self, peer_id: &N::PeerId) {
        self.outdated_peers.remove(peer_id);
        self.incompatible_peers.remove(peer_id);
    }
}

impl<N: Network, M: MacroSync<N::PeerId>, L: LiveSync<N>> Stream for Syncer<N, M, L> {
    type Item = LiveSyncPushEvent;

    // Polls internal components and handles network, macro sync, and live sync events.
    // Also manages peer states by removing incompatible peers and assigning
    // compatible ones to macro or live sync based on the current sync phase.
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        while let Poll::Ready(event) = self.network_events.poll_next_unpin(cx) {
            let event = match event {
                Some(event) => event,
                None => return Poll::Ready(None),
            };
            match event {
                Ok(NetworkEvent::PeerLeft(peer_id)) => self.remove_peer(&peer_id),
                Err(error) => error!(%error, "Syncer lagged receiving network events"),
                _ => {}
            };
        }

        // Poll macro sync and track the peers that it emits.
        while let Poll::Ready(result) = self.macro_sync.poll_next_unpin(cx) {
            match result {
                Some(MacroSyncReturn::Good(peer_id)) => {
                    debug!(%peer_id, "Macro sync returned good peer");
                    self.move_peer_into_live_sync(peer_id);
                }
                Some(MacroSyncReturn::Outdated(peer_id)) => {
                    debug!(%peer_id, "Macro sync returned outdated peer");
                    self.outdated_peers.insert(peer_id);
                }
                Some(MacroSyncReturn::Incompatible(peer_id)) => {
                    debug!(%peer_id, "Macro sync returned incompatible peer");
                    self.check_incompatible_peer(peer_id);
                }
                Some(MacroSyncReturn::Conflicting(peer_id)) => {
                    debug!(%peer_id, "Macro sync returned conflicting peer");
                    let mut syncing_peers = self.live_sync.peers();
                    syncing_peers.push(peer_id);
                    syncing_peers.append(&mut self.macro_sync.drain_peers());

                    self.macro_sync.fallback(syncing_peers);
                }
                None => {}
            }
        }

        // Poll live sync and track the peers that it emits.
        while let Poll::Ready(Some(result)) = self.live_sync.poll_next_unpin(cx) {
            match result {
                LiveSyncEvent::PushEvent(push_event) => {
                    if let LiveSyncPushEvent::AcceptedAnnouncedBlock(..) = push_event {
                        self.accepted_announcements = self.accepted_announcements.saturating_add(1);
                    }
                    return Poll::Ready(Some(push_event));
                }
                LiveSyncEvent::PeerEvent(peer_event) => match peer_event {
                    LiveSyncPeerEvent::Behind(peer_id) => {
                        self.outdated_peers.insert(peer_id);
                    }
                    LiveSyncPeerEvent::Ahead(peer_id) => {
                        self.move_peer_into_macro_sync(peer_id);
                    }
                },
            }
        }

        // Re-check all outdated/incompatible peers whenever the interval triggers.
        while self.check_interval.poll_next_unpin(cx).is_ready() {
            self.check_outdated_peers();
            self.check_incompatible_peers();
        }

        // Handle pending incompatible peer checks.
        while let Poll::Ready(Some((peer_id, synced))) = self.pending_checks.poll_next_unpin(cx) {
            if synced {
                if self.blockchain.read().can_enforce_validity_window() {
                    debug!(%peer_id, "Incompatible peer is in sync, moving it to live sync");
                    self.move_peer_into_live_sync(peer_id);
                } else {
                    debug!(%peer_id, "Incompatible peer is in sync, waiting for the validity sync to complete");
                    self.incompatible_peers.insert(peer_id);
                }
            } else {
                // Peer is still not in sync.
                self.incompatible_peers.insert(peer_id);
            }
        }

        Poll::Pending
    }
}
