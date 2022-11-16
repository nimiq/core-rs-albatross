use crate::sync::follow::request_component::RequestComponent;
use crate::sync::follow::FollowMode;
use crate::sync::history::MacroSyncReturn;
use futures::{Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;
use pin_project::pin_project;
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

pub trait MacroSyncStream<TPeerId>: Stream<Item = MacroSyncReturn<TPeerId>> + Unpin + Send {
    fn add_peer(&self, peer_id: TPeerId);
}

pub trait LiveSyncStream<N: Network>:
    Stream<Item = LiveSyncEvent<N::PeerId>> + Unpin + Send
{
    fn on_block_announced(
        &mut self,
        block: Block,
        peer_id: N::PeerId,
        pubsub_id: Option<<N as Network>::PubsubId>,
    );
    fn add_peer(&mut self, peer_id: N::PeerId);
    fn num_peers(&self) -> usize;
    fn peers(&self) -> Vec<N::PeerId>;
}

#[derive(Clone, Debug)]
pub enum LiveSyncEvent<TPeerId> {
    PushEvent(LiveSyncPushEvent),
    PeerEvent(LiveSyncPeerEvent<TPeerId>),
}

#[derive(Clone, Debug)]
pub enum LiveSyncPushEvent {
    AcceptedAnnouncedBlock(Blake2bHash),
    AcceptedBufferedBlock(Blake2bHash, usize),
    ReceivedMissingBlocks(Blake2bHash, usize),
    RejectedBlock(Blake2bHash),
}

#[derive(Clone, Debug)]
pub enum LiveSyncPeerEvent<TPeerId> {
    OutdatedPeer(TPeerId),
    AdvancedPeer(TPeerId),
}

#[pin_project]
pub struct Syncer<N: Network, TReq: RequestComponent<N>> {
    blockchain: BlockchainProxy,

    network: Arc<N>,

    #[pin]
    pub live_sync: FollowMode<N, TReq>,

    macro_sync: Pin<Box<dyn MacroSyncStream<N::PeerId>>>,

    /// The number of extended blocks through announcements.
    accepted_announcements: usize,

    outdated_peers: HashSet<N::PeerId>,

    outdated_timeouts: HashMap<N::PeerId, Instant>,
}

impl<N: Network, TReq: RequestComponent<N>> Syncer<N, TReq> {
    const CHECK_OUTDATED_TIMEOUT: Duration = Duration::from_secs(20);

    pub async fn new(
        blockchain: BlockchainProxy,
        network: Arc<N>,
        live_sync: FollowMode<N, TReq>,
        history_sync: Pin<Box<dyn MacroSyncStream<N::PeerId>>>,
    ) -> Syncer<N, TReq> {
        Syncer {
            blockchain,
            network,
            live_sync,
            macro_sync: history_sync,
            accepted_announcements: 0,
            outdated_peers: Default::default(),
            outdated_timeouts: Default::default(),
        }
    }

    pub fn push_block(&mut self, block: Block, peer_id: N::PeerId, pubsub_id: Option<N::PubsubId>) {
        self.live_sync.on_block_announced(block, peer_id, pubsub_id);
    }

    fn move_peer_into_history_sync(&mut self, peer_id: N::PeerId) {
        debug!("Adding peer {:?} into history sync", peer_id);
        self.macro_sync.add_peer(peer_id);
    }

    fn move_peer_into_live_sync(&mut self, peer_id: N::PeerId) {
        debug!("Adding peer {:?} into live sync", peer_id);
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
}

impl<N: Network, TReq: RequestComponent<N>> Stream for Syncer<N, TReq> {
    type Item = LiveSyncPushEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Poll self.history_sync and add new peers to self.sync_queue.
        while let Poll::Ready(result) = self.macro_sync.poll_next_unpin(cx) {
            match result {
                Some(MacroSyncReturn::Good(peer_id)) => {
                    self.move_peer_into_live_sync(peer_id);
                }
                Some(MacroSyncReturn::Outdated(peer_id)) => {
                    debug!(
                        "History sync returned outdated peer {:?}. Waiting.",
                        peer_id
                    );
                    self.outdated_timeouts.insert(peer_id, Instant::now());
                    self.outdated_peers.insert(peer_id);
                }
                None => {}
            }
        }

        self.check_peers_up_to_date();

        while let Poll::Ready(Some(result)) = self.as_mut().project().live_sync.poll_next(cx) {
            match result {
                LiveSyncEvent::PushEvent(push_event) => {
                    return Poll::Ready(Some(push_event));
                }
                LiveSyncEvent::PeerEvent(peer_event) => match peer_event {
                    LiveSyncPeerEvent::OutdatedPeer(peer_id) => {
                        self.as_mut().project().outdated_peers.insert(peer_id);
                    }
                    LiveSyncPeerEvent::AdvancedPeer(peer_id) => {
                        self.move_peer_into_history_sync(peer_id);
                    }
                },
            }
        }

        Poll::Pending
    }
}

impl<N: Network, TReq: RequestComponent<N>> Syncer<N, TReq> {
    /// Adds all outdated peers that were checked more than TIMEOUT ago to history sync
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
