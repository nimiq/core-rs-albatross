use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use futures::{future::BoxFuture, FutureExt};
use nimiq_block::Block;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_network_interface::{
    network::{CloseReason, Network, SubscribeEvents},
    request::RequestError,
};
use nimiq_utils::{spawn, stream::FuturesUnordered};

use crate::{
    messages::{BlockError, ResponseHead},
    sync::sync_interface::{MacroSync, PeerMacroRequests},
};

/// The PicoMacroSync is one type of MacroSync that operates on a per peer basis,
/// emitting peers either as Outdated, Good or Conflicting.
/// To do this, it will:
///   1. Request epoch IDs from the peer
///   2. Request the last (if any) election or checkpoint blocks
///   3. Apply the latest state, if any discrepancy is found, then it will emit the peer
///      as conflicting so that we can fallback to other macro sync mechanism.
pub struct PicoMacroSync<TNetwork: Network> {
    /// The blockchain
    pub(crate) blockchain: BlockchainProxy,
    /// Reference to the network
    pub(crate) network: Arc<TNetwork>,
    /// Stream for peer joined and peer left events
    pub(crate) network_event_rx: SubscribeEvents<TNetwork::PeerId>,
    /// Used to track the macro requests on a per peer basis
    pub(crate) peer_requests: HashMap<TNetwork::PeerId, PeerMacroRequests>,
    /// The stream for head requests
    pub(crate) head_stream:
        FuturesUnordered<BoxFuture<'static, Option<(ResponseHead, TNetwork::PeerId)>>>,
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
    /// Collection of peers we are currently pico macro syncing with
    pub(crate) syncing_peers: HashSet<TNetwork::PeerId>,
}

impl<TNetwork: Network> PicoMacroSync<TNetwork> {
    pub fn new(
        blockchain: BlockchainProxy,
        network: Arc<TNetwork>,
        network_event_rx: SubscribeEvents<TNetwork::PeerId>,
    ) -> Self {
        Self {
            blockchain,
            network,
            network_event_rx,
            peer_requests: HashMap::new(),
            head_stream: FuturesUnordered::new(),
            block_headers: Default::default(),
            syncing_peers: HashSet::new(),
        }
    }

    /// This function is used when a peer disconnects, to remove any outstanding request
    pub fn remove_peer_requests(&mut self, peer_id: TNetwork::PeerId) {
        self.peer_requests.remove(&peer_id);
        self.syncing_peers.remove(&peer_id);
    }

    /// Performs cleaning and notifies the network when a peer is disconnected
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

impl<TNetwork: Network> MacroSync<TNetwork::PeerId> for PicoMacroSync<TNetwork> {
    const MAX_REQUEST_EPOCHS: u16 = 1000; // TODO: Use other value

    fn add_peer(&mut self, peer_id: TNetwork::PeerId) {
        info!(%peer_id, "Pico MacroSync: Requesting head from peer");

        self.syncing_peers.insert(peer_id);
        let future = Self::request_head(Arc::clone(&self.network), peer_id).boxed();
        self.head_stream.push(future);
    }

    fn fallback(&mut self, _peers: Vec<TNetwork::PeerId>) {
        unimplemented!()
    }

    fn drain_peers(&mut self) -> Vec<TNetwork::PeerId> {
        self.syncing_peers.drain().collect()
    }
}
