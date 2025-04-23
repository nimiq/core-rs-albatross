//! The `syncer_proxy` module provides an abstraction layer over multiple syncing mechanisms
//! used in Nimiq nodes. Each variant corresponds to a different type of node configuration.
//!
//! - `History`: History node syncs all the historical data.
//! - `Full`: Full node focused on state sync.
//! - `Light`: Light node using macro light sync.
//! - `Pico`: Pico node using minimal sync logic, with fallback.

#[cfg(feature = "full")]
use std::cmp::max;
use std::{
    mem,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::network::{Network, SubscribeEvents};
#[cfg(feature = "full")]
use nimiq_primitives::policy::Policy;
use nimiq_zkp_component::zkp_component::ZKPComponentProxy;
use parking_lot::Mutex;
use pin_project::pin_project;

use super::{
    pico::PicoMacroSync,
    sync_interface::{MacroSync, MacroSyncReturn},
};
#[cfg(feature = "full")]
use crate::sync::{
    history::HistoryMacroSync,
    live::{diff_queue::DiffQueue, state_queue::StateQueue, StateLiveSync},
};
use crate::{
    consensus::ResolveBlockRequest,
    sync::{
        light::LightMacroSync,
        live::{
            block_queue::{BlockQueue, BlockSource},
            queue::QueueConfig,
            BlockLiveSync,
        },
        sync_interface::LiveSyncPushEvent,
        syncer::Syncer,
    },
    BlsCache,
};

macro_rules! gen_syncer_match {
    ($self: ident, $f: ident $(, $arg:expr )*) => {
        match $self {
            #[cfg(feature = "full")]
            SyncerProxy::History(syncer) => syncer.$f($( $arg ),*),
            #[cfg(feature = "full")]
            SyncerProxy::Full(syncer) => syncer.$f($( $arg ),*),
            SyncerProxy::Light(syncer) => syncer.$f($( $arg ),*),
            SyncerProxy::Pico(syncer) => syncer.$f($( $arg ),*),
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
    /// Pico Syncer, uses pico macro sync, light macro sync and block live sync
    Pico(Syncer<N, EitherSyncer<N>, BlockLiveSync<N>>),
}

/// Represents one of two possible macro sync mechanisms: LightMacroSync or PicoMacroSync
/// only one of them can be active at any point in time.
#[pin_project(project = EitherSyncerProj)]
pub enum EitherSyncer<N: Network> {
    Fallback(LightMacroSync<N>),
    Normal(PicoMacroSync<N>),
}

impl<TNetwork: Network> MacroSync<TNetwork::PeerId> for EitherSyncer<TNetwork> {
    const MAX_REQUEST_EPOCHS: u16 = 1000; // TODO: Use other value

    fn add_peer(&mut self, peer_id: TNetwork::PeerId) {
        match self {
            EitherSyncer::Fallback(macro_sync) => macro_sync.add_peer(peer_id),
            EitherSyncer::Normal(macro_sync) => macro_sync.add_peer(peer_id),
        }
    }

    fn fallback(&mut self, peers: Vec<TNetwork::PeerId>) {
        // This functionality is only implemented for switching from Pico to Light macro sync.
        let EitherSyncer::Normal(pico) = self else {
            return;
        };

        log::info!("Falling back to light macro sync");

        // Get all the peers from the pico macro sync move them to light macro sync
        let network = &pico.network;
        let network_event_rx = network.subscribe_events();
        let blockchain = &pico.blockchain;

        match blockchain {
            #[cfg(feature = "full")]
            BlockchainProxy::Full(_) => {
                unimplemented!()
            }
            BlockchainProxy::Light(light_blockchain) => {
                let blockchain = light_blockchain.upgradable_read();
                // Reset the blockchain state
                log::info!("Resetting the light blockchain state");
                LightBlockchain::reset_blockchain(blockchain);
            }
        }

        log::info!("Initializing Light Macro Sync");
        let mut light_macro_sync = LightMacroSync::new(
            pico.blockchain.clone(),
            Arc::clone(&pico.network),
            network_event_rx,
            pico.zkp_component_proxy.clone(),
            0, // Since the light sync does not keep state, we ignore the threshold.
        );

        for peer_id in peers {
            info!(%peer_id, "Adding peer into fallback macro sync");
            light_macro_sync.add_peer(peer_id);
        }

        let _ = mem::replace(self, EitherSyncer::Fallback(light_macro_sync));
    }

    /// Removes all peers from the macro sync and returns their IDs.
    fn drain_peers(&mut self) -> Vec<TNetwork::PeerId> {
        match self {
            EitherSyncer::Fallback(macro_sync) => macro_sync.drain_peers(),
            EitherSyncer::Normal(macro_sync) => macro_sync.drain_peers(),
        }
    }
}

impl<TNetwork: Network> Stream for EitherSyncer<TNetwork> {
    type Item = MacroSyncReturn<TNetwork::PeerId>;

    /// Polls the underlying macro sync variant for the next macro sync result.
    ///
    /// This delegates to the currently active macro sync implementation (either `LightMacroSync`
    /// or `PicoMacroSync`), allowing `EitherSyncer` to behave as a single stream interface.
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.project() {
            EitherSyncerProj::Fallback(macro_sync) => macro_sync.poll_next_unpin(cx),
            EitherSyncerProj::Normal(macro_sync) => macro_sync.poll_next_unpin(cx),
        }
    }
}

impl<N: Network> SyncerProxy<N> {
    #[cfg(feature = "full")]
    /// Creates a new instance of a `SyncerProxy` for the `History` variant
    pub async fn new_history(
        blockchain_proxy: BlockchainProxy,
        network: Arc<N>,
        bls_cache: Arc<Mutex<BlsCache>>,
        network_event_rx: SubscribeEvents<N::PeerId>,
    ) -> Self {
        assert!(
            matches!(blockchain_proxy, BlockchainProxy::Full(_)),
            "History Syncer can only be created for a full blockchain"
        );

        // Extract the full blockchain from the proxy.
        let blockchain = match &blockchain_proxy {
            BlockchainProxy::Full(blockchain) => Arc::clone(blockchain),
            BlockchainProxy::Light(_) => unreachable!(),
        };

        // Initialize the block queue for pending blocks to be processed during sync
        let block_queue = BlockQueue::new(
            Arc::clone(&network),
            blockchain_proxy.clone(),
            QueueConfig::default(),
        )
        .await;

        // Set up the live sync component using the block queue
        let live_sync = BlockLiveSync::with_queue(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            block_queue,
            bls_cache,
        );

        // Set up the macro sync component for historical sync.
        let macro_sync = HistoryMacroSync::new(blockchain, Arc::clone(&network), network_event_rx);

        Self::History(Syncer::new(
            blockchain_proxy,
            network,
            live_sync,
            macro_sync,
        ))
    }

    #[cfg(feature = "full")]
    /// Creates a new instance of a `SyncerProxy` for the `Full` variant
    pub async fn new_full(
        blockchain_proxy: BlockchainProxy,
        network: Arc<N>,
        bls_cache: Arc<Mutex<BlsCache>>,
        zkp_component_proxy: ZKPComponentProxy<N>,
        network_event_rx: SubscribeEvents<N::PeerId>,
        full_sync_threshold: u32,
    ) -> Self {
        // Start from the default configuration for the block queue.
        let mut queue_config = QueueConfig::default();
        // Ensure the queue size is large enough for full sync, based on the given threshold.
        let min_queue_size = full_sync_threshold + Policy::blocks_per_batch() * 2;
        queue_config.window_ahead_max = max(min_queue_size, queue_config.window_ahead_max);
        queue_config.buffer_max = max(min_queue_size as usize, queue_config.buffer_max);

        // Create the block queue used to buffer and manage incoming blocks.
        let block_queue = BlockQueue::new(
            Arc::clone(&network),
            blockchain_proxy.clone(),
            queue_config.clone(),
        )
        .await;

        // Extract the full blockchain reference from the proxy.
        let blockchain = match &blockchain_proxy {
            BlockchainProxy::Full(blockchain) => Arc::clone(blockchain),
            BlockchainProxy::Light(_) => unreachable!(),
        };

        // Create the diff queue, which buffers block differences before applying state changes.
        let diff_queue = DiffQueue::with_block_queue(Arc::clone(&network), block_queue);

        // Create the state queue, which applies state updates based on the diff queue.
        let state_queue = StateQueue::with_diff_queue(
            Arc::clone(&network),
            Arc::clone(&blockchain),
            diff_queue,
            queue_config,
        );

        // Set up live sync with state queue.
        let live_sync = StateLiveSync::with_queue(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            state_queue,
            bls_cache,
        );

        // The task executor that is supplied for the light macro sync variant is tokio
        // because the full sync is not supported in wasm
        let macro_sync = LightMacroSync::new(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            network_event_rx,
            zkp_component_proxy,
            full_sync_threshold,
        );

        // Return the Full node variant of the SyncerProxy.
        Self::Full(Syncer::new(
            blockchain_proxy,
            network,
            live_sync,
            macro_sync,
        ))
    }

    /// Creates a new instance of a `SyncerProxy` for the `Light` variant
    pub async fn new_light(
        blockchain_proxy: BlockchainProxy,
        network: Arc<N>,
        bls_cache: Arc<Mutex<BlsCache>>,
        zkp_component_proxy: ZKPComponentProxy<N>,
        network_event_rx: SubscribeEvents<N::PeerId>,
    ) -> Self {
        // Light sync does not require full block bodies, only headers and metadata.
        let block_queue_config = QueueConfig {
            include_body: false,
            ..Default::default()
        };

        // Create a block queue to manage block announcements and requests.
        let block_queue = BlockQueue::new(
            Arc::clone(&network),
            blockchain_proxy.clone(),
            block_queue_config,
        )
        .await;

        // Live sync is responsible for syncing micro blocks using the block queue.
        let live_sync = BlockLiveSync::with_queue(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            block_queue,
            bls_cache,
        );

        // Set up LightMacroSync, which uses ZKPs to sync the light node to the latest macro block.
        let macro_sync = LightMacroSync::new(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            network_event_rx,
            zkp_component_proxy,
            0, // Since the light sync does not keep state, we ignore the threshold.
        );

        // Return the constructed Light node variant of the SyncerProxy.
        Self::Light(Syncer::new(
            blockchain_proxy,
            network,
            live_sync,
            macro_sync,
        ))
    }

    /// Creates a new instance of a `SyncerProxy` for the `Pico` variant
    pub async fn new_pico(
        blockchain_proxy: BlockchainProxy,
        network: Arc<N>,
        bls_cache: Arc<Mutex<BlsCache>>,
        zkp_component_proxy: ZKPComponentProxy<N>,
        network_event_rx: SubscribeEvents<N::PeerId>,
    ) -> Self {
        // Configure the block queue to exclude block bodies
        let block_queue_config = QueueConfig {
            include_body: false,
            ..Default::default()
        };

        // Set up a block queue to be used by the live sync process.
        let block_queue = BlockQueue::new(
            Arc::clone(&network),
            blockchain_proxy.clone(),
            block_queue_config,
        )
        .await;

        // Create a live sync instance using the configured block queue.
        let live_sync = BlockLiveSync::with_queue(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            block_queue,
            bls_cache,
        );

        // Set up PicoMacroSync
        let pico_macro_sync = PicoMacroSync::new(
            blockchain_proxy.clone(),
            Arc::clone(&network),
            network_event_rx,
            zkp_component_proxy,
        );

        // We start with the Pico Macro sync mechanism; can later fall back to LightMacroSync
        let normal_syncer = EitherSyncer::Normal(pico_macro_sync);

        // Return the SyncerProxy in the Pico node variant.
        Self::Pico(Syncer::new(
            blockchain_proxy,
            network,
            live_sync,
            normal_syncer,
        ))
    }

    /// Pushes a block for the live sync method
    pub fn push_block(&mut self, block: Block, block_source: BlockSource<N>) {
        gen_syncer_match!(self, push_block, block, block_source)
    }

    /// Returns the number of peers doing live synchronization
    pub fn num_peers(&self) -> usize {
        gen_syncer_match!(self, num_peers)
    }

    /// Returns the peers doing live synchronization
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
    /// Triggers resolution of a block.
    pub fn resolve_block(&mut self, request: ResolveBlockRequest<N>) {
        gen_syncer_match!(self, resolve_block, request)
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
            SyncerProxyProj::Pico(syncer) => syncer.poll_next_unpin(cx),
        }
    }
}
