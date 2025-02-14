use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{FutureExt, Stream, StreamExt};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_network_interface::network::{CloseReason, Network, NetworkEvent};
use nimiq_primitives::policy::Policy;

use crate::sync::{
    pico::PicoMacroSync,
    sync_interface::{MacroSync, MacroSyncReturn},
};

impl<TNetwork: Network> PicoMacroSync<TNetwork> {
    // This function is the one that starts the PicoMacroSync process,
    // by adding peers into the MacroSync component.
    // It also removes peers from the internal data structures, when they leave
    fn poll_network_events(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            match result {
                Ok(NetworkEvent::PeerLeft(peer_id)) => {
                    // Remove the peer from internal data structures.
                    self.remove_peer_requests(peer_id);
                }
                Ok(NetworkEvent::PeerJoined(peer_id, _)) => {
                    // Query if that peer provides the necessary services for syncing
                    if self.network.peer_provides_required_services(peer_id) {
                        // Request zkps and start the macro sync process
                        self.add_peer(peer_id);
                    } else {
                        // We can't sync with this peer as it doesn't provide the services that we need.
                        // Emit the peer as incompatible.
                        self.syncing_peers.remove(&peer_id);
                        return Poll::Ready(Some(MacroSyncReturn::Incompatible(peer_id)));
                    }
                }
                Ok(_) => {}
                Err(_) => return Poll::Ready(None),
            }
        }

        Poll::Pending
    }

    fn poll_epoch_ids(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some(Some(epoch_ids))) = self.epoch_ids_stream.poll_next_unpin(cx) {
            // The peer might have disconnected during the request.
            if !self.network.has_peer(epoch_ids.sender) {
                continue;
            }

            // If the peer didn't find any of our locators, we are done with it and emit it.
            if !epoch_ids.locator_found {
                debug!(
                    peer_id = ?epoch_ids.sender,
                    "Peer is behind or on different chain"
                );
                self.syncing_peers.remove(&epoch_ids.sender);
                return Poll::Ready(Some(MacroSyncReturn::Outdated(epoch_ids.sender)));
            } else if epoch_ids.ids.is_empty() && epoch_ids.checkpoint.is_none() {
                // We are synced with this peer.
                debug!(
                    peer_id = ?epoch_ids.sender,
                    "Finished macro syncing with peer");
                self.syncing_peers.remove(&epoch_ids.sender);
                return Poll::Ready(Some(MacroSyncReturn::Good(epoch_ids.sender)));
            }

            // If the macro header process deems a peer useless, it is returned here and we emit it.
            if let Some(agent) = self.request_macro_headers(epoch_ids) {
                self.syncing_peers.remove(&agent);
                return Poll::Ready(Some(MacroSyncReturn::Outdated(agent)));
            }
        }

        Poll::Pending
    }

    fn poll_macro_blocks(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some(result)) = self.block_headers.poll_next_unpin(cx) {
            match result {
                (Ok(Ok(block)), peer_id) => {
                    if let Some(peer_requests) = self.peer_requests.get_mut(&peer_id) {
                        if !peer_requests.update_request(block) {
                            // We received a block we were not expecting from this peer
                            log::warn!(%peer_id,
                                "Banning peer due to a non expected response",
                            );
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }

                        if peer_requests.is_ready() {
                            log::trace!(%peer_id, "All pending requests are ready");

                            while let Some((_, block)) = peer_requests.pop_request() {
                                let block = block.expect("At this point the queue should be ready");

                                if !block.is_macro() {
                                    log::warn!(%peer_id,
                                        "Banning peer because it sent us an invalid block type ",
                                    );
                                    self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                                    return Poll::Ready(None);
                                }

                                // Check if the block is still valid for us or if it is outdated before trying to apply it
                                let push_result = match self.blockchain {
                                    #[cfg(feature = "full")]
                                    BlockchainProxy::Full(_) => {
                                        panic!("Pico macro sync is not supported in the full blockchain")
                                    }
                                    BlockchainProxy::Light(ref light_blockchain) => {
                                        let blockchain = light_blockchain.upgradable_read();
                                        let latest_block_number = blockchain.block_number();

                                        if block.block_number() < latest_block_number {
                                            // The peer is outdated, so we emit it, and we remove it
                                            self.peer_requests.remove(&peer_id);
                                            self.syncing_peers.remove(&peer_id);
                                            return Poll::Ready(Some(MacroSyncReturn::Outdated(
                                                peer_id,
                                            )));
                                        }

                                        if Policy::is_election_block_at(block.block_number()) {
                                            if Policy::genesis_block_number() == latest_block_number
                                            {
                                                log::info!("PicoSync: Pushing latest election");
                                                // If we are currently at the genesis block we blindly push the election block we obtained
                                                LightBlockchain::push_pico_election(
                                                    blockchain,
                                                    block.clone(),
                                                )
                                            } else {
                                                // We don't accept other election blocks if we are not in the genesis.
                                                // Although we could tolerate +/- 1 block differences
                                                self.syncing_peers.remove(&peer_id);
                                                return Poll::Ready(Some(
                                                    MacroSyncReturn::Conflicting(peer_id),
                                                ));
                                            }
                                        } else {
                                            // We only accept macro blocks from the current epoch
                                            if blockchain.epoch_number()
                                                == Policy::epoch_at(latest_block_number)
                                            {
                                                LightBlockchain::push_macro(
                                                    blockchain,
                                                    block.clone(),
                                                )
                                            } else {
                                                self.syncing_peers.remove(&peer_id);
                                                return Poll::Ready(Some(
                                                    MacroSyncReturn::Conflicting(peer_id),
                                                ));
                                            }
                                        }
                                    }
                                };

                                match push_result {
                                    Ok(push_result) => {
                                        log::debug!(
                                            block_number = block.block_number(),
                                            ?push_result,
                                            "Pushed a macro block",
                                        );
                                    }
                                    Err(error) => {
                                        log::warn!(
                                            block_number = block.block_number(),
                                            ?error,
                                            %peer_id,
                                            "Failed to push macro block",
                                        );
                                        // Since we cannot differentiate between a malicious peer and someone who's behind and
                                        // we don't know if the initial state (election block) that we obtained was good
                                        // then we need to fallback to light macro sync.
                                        self.syncing_peers.remove(&peer_id);
                                        return Poll::Ready(Some(MacroSyncReturn::Conflicting(
                                            peer_id,
                                        )));
                                    }
                                }
                            }
                            // At this point we applied all the pending requests from this peer
                            self.peer_requests.remove(&peer_id);

                            // Re-request epoch ids after applying these blocks in order to know if we are up to date with this peer
                            // or if there is more to sync
                            let future = Self::request_epoch_ids(
                                self.blockchain.clone(),
                                Arc::clone(&self.network),
                                peer_id,
                            )
                            .boxed();
                            self.epoch_ids_stream.push(future);
                        }
                    } else {
                        // If we don't have any pending requests from this peer, we proceed requesting epoch ids
                        let future = Self::request_epoch_ids(
                            self.blockchain.clone(),
                            Arc::clone(&self.network),
                            peer_id,
                        )
                        .boxed();
                        self.epoch_ids_stream.push(future);
                    }
                }
                (Ok(Err(error)), peer_id) => {
                    trace!(%error, %peer_id, "Received a response for a failed request on the remote side");
                    // If a block request fails, we disconnect from this peer
                    self.disconnect_peer(peer_id, CloseReason::Error);
                }
                (Err(error), peer_id) => {
                    trace!(?error, %peer_id, "Failed block request");
                    // If a block request fails, we disconnect from this peer
                    self.disconnect_peer(peer_id, CloseReason::Error);
                }
            }
        }

        Poll::Pending
    }
}

impl<TNetwork: Network> Stream for PicoMacroSync<TNetwork> {
    type Item = MacroSyncReturn<TNetwork::PeerId>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(o) = self.poll_network_events(cx) {
            return Poll::Ready(o);
        }

        if let Poll::Ready(o) = self.poll_epoch_ids(cx) {
            return Poll::Ready(o);
        }

        if let Poll::Ready(o) = self.poll_macro_blocks(cx) {
            return Poll::Ready(o);
        }

        Poll::Pending
    }
}
