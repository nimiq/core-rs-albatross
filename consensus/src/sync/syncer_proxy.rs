use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{Stream, StreamExt};
use nimiq_zkp_component::zkp_component::ZKPComponentProxy;
use parking_lot::Mutex;

use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_network_interface::network::{Network, SubscribeEvents};
use pin_project::pin_project;

use crate::sync::{
    history::HistoryMacroSync,
    live::{
        block_queue::{block_request_component::BlockRequestComponent, BlockQueue},
        queue::QueueConfig,
        BlockLiveSync,
    },
    syncer::{LiveSyncPushEvent, Syncer},
};

use super::light::LightMacroSync;

macro_rules! gen_syncer_match {
    ($self: ident, $f: ident $(, $arg:expr )*) => {
        match $self {
            SyncerProxy::History(syncer) => syncer.$f($( $arg ),*),
            SyncerProxy::Light(syncer) => syncer.$f($( $arg ),*),
        }
    };
}

#[pin_project(project = SyncerProxyProj)]
/// The `SyncerProxy` is an abstraction over multiple types of `Syncer`s.
pub enum SyncerProxy<N: Network> {
    /// History Syncer, uses history macro sync for macro sync and block live sync.
    History(Syncer<N, HistoryMacroSync<N>, BlockLiveSync<N, BlockRequestComponent<N>>>),
    /// Light Syncer, uses light macro sync for macro sync and block live sync.
    Light(Syncer<N, LightMacroSync<N>, BlockLiveSync<N, BlockRequestComponent<N>>>),
}

impl<N: Network> SyncerProxy<N> {
    #[cfg(not(target_family = "wasm"))]
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
                let request_component = BlockRequestComponent::new(
                    network.subscribe_events(),
                    Arc::clone(&network),
                    true,
                );

                let block_queue = BlockQueue::new(
                    Arc::clone(&network),
                    blockchain_proxy.clone(),
                    request_component,
                    QueueConfig::default(),
                )
                .await;

                let live_sync = BlockLiveSync::with_queue(
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

    /// Creates a new instance of a `SyncerProxy` for the `Light` variant
    pub async fn new_light(
        blockchain_proxy: BlockchainProxy,
        network: Arc<N>,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        zkp_component_proxy: Arc<ZKPComponentProxy<N>>,
        network_event_rx: SubscribeEvents<N::PeerId>,
    ) -> Self {
        assert!(
            matches!(blockchain_proxy, BlockchainProxy::Light(_)),
            "Light Syncer can only be created for a light blockchain"
        );

        match blockchain_proxy {
            BlockchainProxy::Light(ref blockchain) => {
                let request_component = BlockRequestComponent::new(
                    network.subscribe_events(),
                    Arc::clone(&network),
                    false,
                );

                let block_queue_config = QueueConfig {
                    include_micro_bodies: false,
                    ..Default::default()
                };

                let block_queue = BlockQueue::new(
                    Arc::clone(&network),
                    blockchain_proxy.clone(),
                    request_component,
                    block_queue_config,
                )
                .await;

                let live_sync = BlockLiveSync::with_queue(
                    blockchain_proxy.clone(),
                    Arc::clone(&network),
                    block_queue,
                    bls_cache,
                );

                let macro_sync = LightMacroSync::new(
                    Arc::clone(blockchain),
                    network,
                    network_event_rx,
                    zkp_component_proxy,
                );

                Self::Light(Syncer::new(live_sync, macro_sync))
            }
            BlockchainProxy::Full(_) => unreachable!(),
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

    /// Returns whether the state sync has finished (or `true` if there is no state sync required)
    pub fn state_complete(&self) -> bool {
        gen_syncer_match!(self, state_complete)
    }
}

impl<N: Network> Stream for SyncerProxy<N> {
    type Item = LiveSyncPushEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match self.project() {
            SyncerProxyProj::History(syncer) => syncer.poll_next_unpin(cx),
            SyncerProxyProj::Light(syncer) => syncer.poll_next_unpin(cx),
        }
    }
}
