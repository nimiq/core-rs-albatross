use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use futures::{future::BoxFuture, FutureExt};
use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{CloseReason, Network, SubscribeEvents},
    request::RequestError,
};
use nimiq_utils::{spawn, stream::FuturesUnordered};
#[cfg(feature = "full")]
use parking_lot::RwLock;

use crate::{
    messages::BlockError,
    sync::sync_interface::{EpochIds, MacroSync, PeerMacroRequests},
};
#[cfg(feature = "full")]
use crate::{
    messages::{HistoryChunk, HistoryChunkError, RequestHistoryChunk},
    sync::{peer_list::PeerList, sync_queue::SyncQueue},
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

#[cfg(feature = "full")]
/// Validity sync queue pending size
const PENDING_SIZE: usize = 5;

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
    /// Used to track the macro requests on a per peer basis
    pub(crate) peer_requests: HashMap<TNetwork::PeerId, PeerMacroRequests>,
    /// The stream for epoch ids requests
    pub(crate) epoch_ids_stream:
        FuturesUnordered<BoxFuture<'static, Option<EpochIds<TNetwork::PeerId>>>>,
    /// Block requests
    pub(crate) block_headers: FuturesUnordered<
        BoxFuture<
            'static,
            (
                Result<Result<Block, BlockError>, RequestError>,
                TNetwork::PeerId,
            ),
        >,
    >,

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
    ) -> Self {
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
            peer_requests: HashMap::new(),
            epoch_ids_stream: FuturesUnordered::new(),
            #[cfg(feature = "full")]
            full_sync_threshold,
            block_headers: Default::default(),
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

    pub fn remove_peer_requests(&mut self, peer_id: TNetwork::PeerId) {
        self.peer_requests.remove(&peer_id);
    }

    pub fn disconnect_peer(&mut self, peer_id: TNetwork::PeerId, reason: CloseReason) {
        // Remove all pending peer requests (if any)
        self.remove_peer_requests(peer_id);
        let network = Arc::clone(&self.network);
        // We disconnect from this peer
        spawn(Box::pin({
            async move {
                network.disconnect_peer(peer_id, reason).await;
            }
        }));
    }
}

impl<TNetwork: Network> MacroSync<TNetwork::PeerId> for LightMacroSync<TNetwork> {
    const MAX_REQUEST_EPOCHS: u16 = 1000; // TODO: Use other value

    fn add_peer(&mut self, peer_id: TNetwork::PeerId) {
        info!(%peer_id, "Requesting epoch ids from peer");
        let future =
            Self::request_epoch_ids(self.blockchain.clone(), Arc::clone(&self.network), peer_id)
                .boxed();
        self.epoch_ids_stream.push(future);
    }

    fn fallback(&mut self, _peers: Vec<TNetwork::PeerId>) {
        unimplemented!()
    }

    fn drain_peers(&mut self) -> Vec<TNetwork::PeerId> {
        unimplemented!()
    }
}
