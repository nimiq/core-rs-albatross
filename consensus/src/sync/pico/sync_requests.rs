use std::sync::Arc;

use futures::FutureExt;
use nimiq_block::Block;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::{CloseReason, Network},
    request::RequestError,
};

use crate::{
    messages::{
        BlockError, MacroChain, MacroChainError, RequestHead, RequestMacroChain, ResponseHead,
    },
    sync::{pico::PicoMacroSync, sync_interface::PeerMacroRequests},
};

impl<TNetwork: Network> PicoMacroSync<TNetwork> {
    /// Starts the sync process by requesting the peer's head
    pub(crate) async fn request_head(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
    ) -> Option<(ResponseHead, TNetwork::PeerId)> {
        let result = network
            .request::<RequestHead>(RequestHead {}, peer_id)
            .await;

        match result {
            Ok(response_head) => Some((response_head, peer_id)),
            Err(error) => {
                log::warn!(%error, %peer_id, "Request head failed");
                network.disconnect_peer(peer_id, CloseReason::Error).await;
                None
            }
        }
    }

    /// Requests macro headers to the current syncing peer
    pub(crate) fn request_macro_headers(
        &mut self,
        peer_head: ResponseHead,
        peer_id: TNetwork::PeerId,
    ) {
        let mut peer_requests = PeerMacroRequests::new();

        let last_election = peer_head.election_hash.clone();

        log::debug!(
            %last_election,
            %peer_id,
            "Request latest election block",
        );

        peer_requests.push_request(last_election.clone());

        let last_election = last_election.clone();
        let network = Arc::clone(&self.network);

        self.block_headers.push(
            async move {
                (
                    Self::request_macro_block(network, peer_id, last_election).await,
                    peer_id,
                )
            }
            .boxed(),
        );

        // Request the latest checkpoint (if any)
        if peer_head.election_hash != peer_head.macro_hash {
            let block_hash = peer_head.macro_hash;
            let network = Arc::clone(&self.network);

            peer_requests.push_request(block_hash.clone());
            self.block_headers.push(
                async move {
                    (
                        Self::request_macro_block(network, peer_id, block_hash).await,
                        peer_id,
                    )
                }
                .boxed(),
            );
        }

        self.peer_requests.insert(peer_id, peer_requests);
    }

    /// Requests a single macro block to the given peer
    pub async fn request_macro_block(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        hash: Blake2bHash,
    ) -> Result<Result<Block, BlockError>, RequestError> {
        // Macro sync only needs the header, so the body is not requested
        crate::sync::request_block(network, peer_id, hash, false).await
    }

    /// Requests the macro chain to the given peer.
    pub async fn request_macro_chain(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        locators: Vec<Blake2bHash>,
        max_epochs: u16,
    ) -> Result<Result<MacroChain, MacroChainError>, RequestError> {
        network
            .request::<RequestMacroChain>(
                RequestMacroChain {
                    locators,
                    max_epochs,
                },
                peer_id,
            )
            .await
    }
}
