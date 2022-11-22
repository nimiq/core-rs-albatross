use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};
use parking_lot::Mutex;

use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_network_interface::network::{Network, SubscribeEvents};
use pin_project::pin_project;

use crate::sync::{
    history::HistoryMacroSync,
    live::{
        block_queue::{BlockQueue, BlockQueueConfig},
        request_component::BlockRequestComponent,
        BlockLiveSync,
    },
    syncer::{LiveSyncPushEvent, Syncer},
};

macro_rules! gen_syncer_match {
    ($self: ident, $f: ident $(, $arg:expr )*) => {
        match $self {
            SyncerProxy::History(syncer) => syncer.$f($( $arg ),*),
        }
    };
}

#[pin_project(project = SyncerProxyProj)]
/// The `SyncerProxy` is an abstraction over multiple types of `Syncer`s.
pub enum SyncerProxy<N: Network> {
    /// History Syncer, uses history macro sync for macro sync and block live sync.
    History(Syncer<N, HistoryMacroSync<N>, BlockLiveSync<N, BlockRequestComponent<N>>>),
}

impl<N: Network> SyncerProxy<N> {
    /// Creates a new instance of a `SyncerProxy` for the `History` variant
    pub async fn new_history(
        blockchain_proxy: BlockchainProxy,
        network: Arc<N>,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        network_event_rx: SubscribeEvents<N::PeerId>,
    ) -> Self {
        assert!(
            matches!(blockchain_proxy, BlockchainProxy::Full(_)),
            "History Syncer can only be created for a full blockchain"
        );

        match blockchain_proxy {
            BlockchainProxy::Full(ref blockchain) => {
                let request_component =
                    BlockRequestComponent::new(network.subscribe_events(), Arc::clone(&network));

                let block_queue = BlockQueue::new(
                    Arc::clone(&network),
                    blockchain_proxy.clone(),
                    request_component,
                    BlockQueueConfig::default(),
                )
                .await;

                let live_sync = BlockLiveSync::new(
                    blockchain_proxy.clone(),
                    Arc::clone(&network),
                    block_queue,
                    bls_cache,
                );

                let macro_sync =
                    HistoryMacroSync::new(Arc::clone(blockchain), network, network_event_rx);

                Self::History(Syncer::new(live_sync, macro_sync))
            }
            BlockchainProxy::Light(_) => unreachable!(),
        }
    }

    /// Pushes a block for the live sync method
    pub fn push_block(&mut self, block: Block, peer_id: N::PeerId, pubsub_id: Option<N::PubsubId>) {
        gen_syncer_match!(self, push_block, block, peer_id, pubsub_id)
    }

    /// Returns the number of peers doing live synchronization
    pub fn num_peers(&self) -> usize {
        gen_syncer_match!(self, num_peers)
    }

    /// Returns the peers of peers doing live synchronization
    pub fn peers(&self) -> Vec<N::PeerId> {
        gen_syncer_match!(self, peers)
    }

    /// Returns the number of accepted block announcements seen
    pub fn accepted_block_announcements(&self) -> usize {
        gen_syncer_match!(self, accepted_block_announcements)
    }
}

impl<N: Network> Stream for SyncerProxy<N> {
    type Item = LiveSyncPushEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match self.project() {
            SyncerProxyProj::History(syncer) => syncer.poll_next_unpin(cx),
        }
    }
}
