use std::sync::Arc;

use futures::FutureExt;
use parking_lot::RwLock;

use nimiq_block::Block;
use nimiq_blockchain::AbstractBlockchain;
use nimiq_hash::Blake2bHash;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::{
    network::Network,
    peer::CloseReason,
    request::{
        InboundRequestError::SenderFutureDropped, RequestError, RequestError::InboundRequest,
    },
};
use nimiq_primitives::policy::Policy;
use nimiq_zkp_component::{
    types::{Error, ZKPRequestEvent},
    zkp_component::ZKPComponentProxy,
};

use crate::messages::{MacroChain, RequestBlock, RequestMacroChain};
use crate::sync::light::{
    sync::{EpochIds, PeerMacroRequests},
    LightMacroSync,
};

impl<TNetwork: Network> LightMacroSync<TNetwork> {
    pub(crate) async fn request_zkps(
        zkp_component: Arc<ZKPComponentProxy<TNetwork>>,
        peer_id: TNetwork::PeerId,
    ) -> (Result<ZKPRequestEvent, Error>, TNetwork::PeerId) {
        let zkp_result = zkp_component.request_zkp_from_peer(peer_id, true).await;

        let (zkp_result, peer_id) = match zkp_result {
            (Ok(zkp_result), peer_id) => (zkp_result, peer_id),
            (Err(error), peer_id) => {
                log::debug!(?error, %peer_id, "Error from channel");

                (
                    Err(Error::Request(InboundRequest(SenderFutureDropped))),
                    peer_id,
                )
            }
        };
        (zkp_result, peer_id)
    }

    pub(crate) async fn request_epoch_ids(
        blockchain: Arc<RwLock<LightBlockchain>>,
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
    ) -> Option<EpochIds<TNetwork::PeerId>> {
        let (locators, epoch_number) = {
            // Order matters here. The first hash found by the recipient of the request will be
            // used, so they need to be in backwards block height order.
            let blockchain = blockchain.read();
            let election_head = blockchain.election_head();
            let macro_head = blockchain.macro_head();

            // So if there is a checkpoint hash that should be included in addition to the election
            // block hash, it should come first.
            let mut locators = vec![];
            if macro_head.hash() != election_head.hash() {
                locators.push(macro_head.hash());
            }
            // The election block is at the end here
            locators.push(election_head.hash());

            (locators, election_head.epoch_number())
        };

        let result = Self::request_macro_chain(
            Arc::clone(&network),
            peer_id,
            locators,
            1000, // TODO: Use other value
        )
        .await;

        match result {
            Ok(macro_chain) => {
                if macro_chain.epochs.is_none() {
                    return Some(EpochIds {
                        locator_found: false,
                        ids: Vec::new(),
                        checkpoint: None,
                        first_epoch_number: 0,
                        sender: peer_id,
                    });
                }

                let epoch_ids = macro_chain.epochs.unwrap();

                // Sanity-check checkpoint block number:
                //  * is in checkpoint epoch
                //  * is a non-election macro block
                if let Some(checkpoint) = &macro_chain.checkpoint {
                    let checkpoint_epoch = epoch_number + epoch_ids.len() as u32 + 1;
                    if Policy::epoch_at(checkpoint.block_number) != checkpoint_epoch
                        || !Policy::is_macro_block_at(checkpoint.block_number)
                        || Policy::is_election_block_at(checkpoint.block_number)
                    {
                        // Peer provided an invalid checkpoint block number, close connection.
                        log::error!(
                            block_number = checkpoint.block_number,
                            checkpoint_epoch = checkpoint_epoch,
                            "Request macro chain failed: invalid checkpoint"
                        );
                        network.disconnect_peer(peer_id, CloseReason::Other).await;
                        return None;
                    }
                }

                log::debug!(
                    received_epochs =  epoch_ids.len(),
                    start_epoch = epoch_number + 1,
                    checkpoint = macro_chain.checkpoint.is_some(),
                    sender = %peer_id,
                    "Received epoch_ids"
                );

                Some(EpochIds {
                    locator_found: true,
                    ids: epoch_ids,
                    checkpoint: macro_chain.checkpoint,
                    first_epoch_number: epoch_number as usize + 1,
                    sender: peer_id,
                })
            }
            Err(error) => {
                log::error!(%error, "Request macro chain failed");
                network.disconnect_peer(peer_id, CloseReason::Other).await;
                None
            }
        }
    }

    pub(crate) fn request_macro_headers(
        &mut self,
        mut epoch_ids: EpochIds<TNetwork::PeerId>,
    ) -> Option<TNetwork::PeerId> {
        // Read our current blockchain state.
        let (our_epoch_id, our_epoch_number, our_block_number) = {
            let blockchain = self.blockchain.read();
            (
                blockchain.election_head_hash(),
                blockchain.election_head().epoch_number() as usize,
                blockchain.block_number(),
            )
        };

        // Truncate epoch_ids by epoch_number: Discard all epoch_ids prior to our accepted state.
        if !epoch_ids.ids.is_empty() && epoch_ids.first_epoch_number <= our_epoch_number {
            let peers_epoch_number = epoch_ids.last_epoch_number();
            if peers_epoch_number < our_epoch_number
                || (peers_epoch_number == our_epoch_number && epoch_ids.checkpoint.is_none())
            {
                // Peer is behind, emit it as useless.
                debug!(
                    our_epoch_number,
                    peers_epoch_number,
                    peer = %epoch_ids.sender,
                    "Peer is behind"
                );
                return Some(epoch_ids.sender);
            } else {
                // Check that the epoch_id sent by the peer at our current epoch number corresponds to
                // our accepted state. If it doesn't, the peer is on a "permanent" fork, so we ban it.
                let peers_epoch_id =
                    &epoch_ids.ids[our_epoch_number - epoch_ids.first_epoch_number];
                if our_epoch_id != *peers_epoch_id {
                    // TODO Actually ban the peer.
                    debug!(
                        our_epoch_number,
                        %our_epoch_id,
                        %peers_epoch_id,
                        peer = %epoch_ids.sender,
                        "Peer is on a different chain"
                    );
                    return Some(epoch_ids.sender);
                }

                epoch_ids.ids = epoch_ids
                    .ids
                    .split_off(our_epoch_number - epoch_ids.first_epoch_number + 1);
                epoch_ids.first_epoch_number = our_epoch_number + 1;
            }
        }

        // Discard checkpoint block if it is old.
        if let Some(checkpoint) = &epoch_ids.checkpoint {
            if checkpoint.block_number <= our_block_number {
                epoch_ids.checkpoint = None;
            }
        }

        let mut peer_requests = PeerMacroRequests::new();

        // Request the election blocks
        for block_hash in epoch_ids.ids {
            let network = Arc::clone(&self.network);
            let peer_id = epoch_ids.sender;

            peer_requests.push_request(block_hash);

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

        // Request the checkpoint (if any)
        if let Some(checkpoint) = &epoch_ids.checkpoint {
            let block_hash = checkpoint.clone().hash;
            let network = Arc::clone(&self.network);
            let peer_id = epoch_ids.sender;
            peer_requests.push_request(block_hash);
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

        self.peer_requests.insert(epoch_ids.sender, peer_requests);

        None
    }

    pub async fn request_macro_block(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        hash: Blake2bHash,
    ) -> Result<Option<Block>, RequestError> {
        // We will only request macro blocks, so we always need the body
        network
            .request::<RequestBlock>(
                RequestBlock {
                    hash,
                    include_micro_bodies: false,
                },
                peer_id,
            )
            .await
    }

    pub async fn request_macro_chain(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        locators: Vec<Blake2bHash>,
        max_epochs: u16,
    ) -> Result<MacroChain, RequestError> {
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
