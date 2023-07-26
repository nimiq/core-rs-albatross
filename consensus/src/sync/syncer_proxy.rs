#[cfg(feature = "full")]
use std::cmp::max;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_network_interface::network::{Network, SubscribeEvents};
use nimiq_primitives::{policy::Policy, task_executor::TaskExecutor};
use nimiq_zkp_component::zkp_component::ZKPComponentProxy;
use parking_lot::Mutex;
use pin_project::pin_project;

#[cfg(feature = "full")]
use crate::sync::{
    history::HistoryMacroSync,
    live::{diff_queue::DiffQueue, state_queue::StateQueue, StateLiveSync},
};
use crate::sync::{
    light::LightMacroSync,
    live::{block_queue::BlockQueue, queue::QueueConfig, BlockLiveSync},
    syncer::{LiveSyncPushEvent, Syncer},
};

macro_rules! gen_syncer_match {
    ($self: ident, $f: ident $(, $arg:expr )*) => {
        match $self {
            #[cfg(feature = "full")]
            SyncerProxy::History(syncer) => syncer.$f($( $arg ),*),
            #[cfg(feature = "full")]
            SyncerProxy::Full(syncer) => syncer.$f($( $arg ),*),
            SyncerProxy::Light(syncer) => syncer.$f($( $arg ),*),
        }
    };
}

#[pin_project(project = SyncerProxyProj)]
/// The `SyncerProxy` is an abstraction over multiple types of `Syncer`s.
pub enum SyncerProxy<N: Network> {
    #[cfg(feature = "full")]
    /// History Syncer, uses history macro sync for macro sync and block live sync.
    History(Syncer<N, HistoryMacroSync<N>, BlockLiveSync<N>>),
    #[cfg(feature = "full")]
    /// Full Syncer, uses light macro sync for macro sync and state live sync.
    Full(Syncer<N, LightMacroSync<N>, StateLiveSync<N>>),
    /// Light Syncer, uses light macro sync for macro sync and block live sync.
    Light(Syncer<N, LightMacroSync<N>, BlockLiveSync<N>>),
}

impl<N: Network> SyncerProxy<N> {
    #[cfg(feature = "full")]
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
                let block_queue = BlockQueue::new(
                    Arc::clone(&network),
                    blockchain_proxy.clone(),
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

    #[cfg(feature = "full")]
    /// Creates a new instance of a `SyncerProxy` for the `Full` variant
    pub async fn new_full(
        blockchain_proxy: BlockchainProxy,
        network: Arc<N>,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        zkp_component_proxy: ZKPComponentProxy<N>,
        network_event_rx: SubscribeEvents<N::PeerId>,
        full_sync_threshold: u32,
    ) -> Self {
        let mut queue_config = QueueConfig::default();
        let min_queue_size = full_sync_threshold + Policy::blocks_per_batch() * 2;
        queue_config.window_ahead_max = max(min_queue_size, queue_config.window_ahead_max);
        queue_config.buffer_max = max(min_queue_size as usize, queue_config.buffer_max);

        let block_queue = BlockQueue::new(
            Arc::clone(&network),
            blockchain_proxy.clone(),
            queue_config.clone(),
        )
        .await;

        let blockchain = match &blockchain_proxy {
            BlockchainProxy::Full(blockchain) => Arc::clone(blockchain),
            BlockchainProxy::Light(_) => unreachable!(),
        };

        let diff_queue = DiffQueue::with_block_queue(Arc::clone(&network), block_queue);

        let state_queue = StateQueue::with_diff_queue(
            Arc::clone(&network),
            Arc::clone(&blockchain),
            diff_queue,
            queue_config,
        );

        let live_sync = StateLiveSync::with_queue(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            state_queue,
            bls_cache,
        );

        // The task executor that is supplied for the light macro sync variant is tokio
        // because the full sync is not supported in wasm
        let macro_sync = LightMacroSync::new(
            blockchain_proxy,
            network,
            network_event_rx,
            zkp_component_proxy,
            full_sync_threshold,
            Box::new(|fut| {
                tokio::spawn(fut);
            }),
        );

        Self::Full(Syncer::new(live_sync, macro_sync))
    }

    /// Creates a new instance of a `SyncerProxy` for the `Light` variant
    pub async fn new_light(
        blockchain_proxy: BlockchainProxy,
        network: Arc<N>,
        bls_cache: Arc<Mutex<PublicKeyCache>>,
        zkp_component_proxy: ZKPComponentProxy<N>,
        network_event_rx: SubscribeEvents<N::PeerId>,
        executor: impl TaskExecutor + Send + 'static,
    ) -> Self {
        let block_queue_config = QueueConfig {
            include_micro_bodies: false,
            ..Default::default()
        };

        let block_queue = BlockQueue::new(
            Arc::clone(&network),
            blockchain_proxy.clone(),
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
            blockchain_proxy,
            network,
            network_event_rx,
            zkp_component_proxy,
            0, // Since the light sync does not keep state, we ignore the threshold.
            executor,
        );

        Self::Light(Syncer::new(live_sync, macro_sync))
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
            #[cfg(feature = "full")]
            SyncerProxyProj::History(syncer) => syncer.poll_next_unpin(cx),
            #[cfg(feature = "full")]
            SyncerProxyProj::Full(syncer) => syncer.poll_next_unpin(cx),
            SyncerProxyProj::Light(syncer) => syncer.poll_next_unpin(cx),
        }
    }
}
