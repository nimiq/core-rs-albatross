use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{Stream, StreamExt};
use instant::Instant;
use nimiq_block::Block;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;

/// Trait that defines how a node synchronizes macro blocks
/// The expected functionality is that there could be different methods of syncing but they
/// all must synchronize to the latest macro block. An implementation of this trait requests,
/// process and validates these macro blocks.
/// The quantity of macro blocks needed or extra metadata associated are defined by each
/// implementor of these trait.
pub trait MacroSync<TPeerId>: Stream<Item = MacroSyncReturn<TPeerId>> + Unpin + Send {
    /// Adds a peer to synchronize macro blocks
    fn add_peer(&self, peer_id: TPeerId);
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
    fn push_block(
        &mut self,
        block: Block,
        peer_id: N::PeerId,
        pubsub_id: Option<<N as Network>::PubsubId>,
    );
    /// Adds a peer to receive or request blocks from it.
    fn add_peer(&mut self, peer_id: N::PeerId);
    /// Returns the number of peers that are being sync'ed with
    fn num_peers(&self) -> usize;
    /// Returns the list of peers that are being sync'ed with
    fn peers(&self) -> Vec<N::PeerId>;
    /// Returns whether the state sync has finished (or `true` if there is no state sync required)
    fn state_complete(&self) -> bool {
        true
    }
}

#[derive(Debug, PartialEq, Eq)]
/// Return type for a `MacroSync`
pub enum MacroSyncReturn<T> {
    /// Macro Sync returned a good type
    Good(T),
    /// Macro Sync returned an outdated type
    Outdated(T),
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
    /// Missing blocks were received
    ReceivedMissingBlocks(Blake2bHash, usize),
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

/// Syncer is the main synchronization object inside `Consensus`
/// It has a reference to the main blockchain and network and has two dynamic
/// trait objects:
/// - Macro sync: Synchronizes up to the last macro block
/// - Live Sync: Synchronizes the blockchain to the blocks being processed/announced
///   by the peers.
/// These two dynamic trait objects are necessary to implement the different types of
/// synchronization such as: history sync, full sync and light sync.
/// The Syncer handles the interactions between these trait objects, the blockchain and
/// the network.
pub struct Syncer<N: Network, M: MacroSync<N::PeerId>, L: LiveSync<N>> {
    /// Synchronizes the blockchain to the blocks being processed/announced by the peers
    pub live_sync: L,

    /// Synchronizes the blockchain to the latest macro blocks of the peers
    pub macro_sync: M,

    /// The number of extended blocks through announcements
    accepted_announcements: usize,

    /// Hash set with the set of peers that have been qualified as outdated
    outdated_peers: HashSet<N::PeerId>,

    /// Hash set with the peer ID as key containing the elapsed time when a peer
    /// was qualified as outdated
    outdated_timeouts: HashMap<N::PeerId, Instant>,
}

impl<N: Network, M: MacroSync<N::PeerId>, L: LiveSync<N>> Syncer<N, M, L> {
    const CHECK_OUTDATED_TIMEOUT: Duration = Duration::from_secs(20);

    pub fn new(live_sync: L, macro_sync: M) -> Syncer<N, M, L> {
        Syncer {
            live_sync,
            macro_sync,
            accepted_announcements: 0,
            outdated_peers: Default::default(),
            outdated_timeouts: Default::default(),
        }
    }

    pub fn push_block(&mut self, block: Block, peer_id: N::PeerId, pubsub_id: Option<N::PubsubId>) {
        self.live_sync.push_block(block, peer_id, pubsub_id);
    }

    fn move_peer_into_macro_sync(&mut self, peer_id: N::PeerId) {
        debug!(%peer_id, "Adding peer to macro sync");
        self.macro_sync.add_peer(peer_id);
    }

    pub fn move_peer_into_live_sync(&mut self, peer_id: N::PeerId) {
        debug!(%peer_id, "Adding peer to live sync");
        self.live_sync.add_peer(peer_id);
    }

    pub fn num_peers(&self) -> usize {
        self.live_sync.num_peers()
    }

    pub fn peers(&self) -> Vec<N::PeerId> {
        self.live_sync.peers()
    }

    pub fn accepted_block_announcements(&self) -> usize {
        self.accepted_announcements
    }

    pub fn state_complete(&self) -> bool {
        self.live_sync.state_complete()
    }
}

impl<N: Network, M: MacroSync<N::PeerId>, L: LiveSync<N>> Stream for Syncer<N, M, L> {
    type Item = LiveSyncPushEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Poll self.history_sync and add new peers to self.sync_queue.
        while let Poll::Ready(result) = self.macro_sync.poll_next_unpin(cx) {
            match result {
                Some(MacroSyncReturn::Good(peer_id)) => {
                    debug!(%peer_id, "Macro sync returned good peer");
                    self.move_peer_into_live_sync(peer_id);
                }
                Some(MacroSyncReturn::Outdated(peer_id)) => {
                    debug!(%peer_id,"Macro sync returned outdated peer. Waiting.");
                    self.outdated_timeouts.insert(peer_id, Instant::now());
                    self.outdated_peers.insert(peer_id);
                }
                None => {}
            }
        }

        self.check_peers_up_to_date();

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
                        self.outdated_timeouts.insert(peer_id, Instant::now());
                        self.outdated_peers.insert(peer_id);
                    }
                    LiveSyncPeerEvent::Ahead(peer_id) => {
                        self.move_peer_into_macro_sync(peer_id);
                    }
                },
            }
        }

        Poll::Pending
    }
}

impl<N: Network, M: MacroSync<N::PeerId>, L: LiveSync<N>> Syncer<N, M, L> {
    /// Adds all outdated peers that were checked more than TIMEOUT ago to macro sync
    fn check_peers_up_to_date(&mut self) {
        let mut peers_todo = Vec::new();
        self.outdated_timeouts.retain(|&peer_id, last_checked| {
            let timeouted = last_checked.elapsed() >= Self::CHECK_OUTDATED_TIMEOUT;
            if timeouted {
                peers_todo.push(peer_id);
            }
            !timeouted
        });
        for peer_id in peers_todo {
            debug!("Adding outdated peer {:?} to history sync", peer_id);
            let peer_id = self.outdated_peers.take(&peer_id).unwrap();
            self.macro_sync.add_peer(peer_id);
        }
    }
}
