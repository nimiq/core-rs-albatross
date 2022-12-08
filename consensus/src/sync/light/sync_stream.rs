use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{FutureExt, Stream, StreamExt};

use nimiq_blockchain::AbstractBlockchain;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_macros::store_waker;
use nimiq_network_interface::network::{Network, NetworkEvent};
use nimiq_zkp_component::types::ZKPRequestEvent::{OutdatedProof, Proof};

use crate::sync::{
    light::LightMacroSync,
    syncer::{MacroSync, MacroSyncReturn},
};

impl<TNetwork: Network> LightMacroSync<TNetwork> {
    // This function is the one that starts the LightMacroSync process,
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
                Ok(NetworkEvent::PeerJoined(peer_id)) => {
                    // Request zkps and start the macro sync process
                    self.add_peer(peer_id);
                }
                Err(_) => return Poll::Ready(None),
            }
        }

        Poll::Pending
    }

    // Function that polls ZKP proofs, there can be several cases to be taken into consideration:
    //   A) The peer never sends the proof or the request fails:
    //         In this case we disconnect the peer
    //   B) The peer does not have a newer proof than the one we already have:
    //         In this case we request epoch ids from this peer
    //   C) The peer has a more recent proof than the one we already have:
    //         In this case we apply the proof to our blockchain and proceed to request epoch ids from this peer
    fn poll_zkps(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        let zkp_request_result = match self.zkp_requests.poll_next_unpin(cx) {
            Poll::Ready(Some(zkp_request_result)) => zkp_request_result,
            _ => return Poll::Pending,
        };

        match zkp_request_result {
            (Ok(zkp_event), peer_id) => match zkp_event {
                Proof { proof, block } => {
                    // Apply a newer proof to the blockchain
                    let result = LightBlockchain::push_zkp(
                        self.blockchain.upgradable_read(),
                        nimiq_block::Block::Macro(block),
                        proof.proof.expect("Expected a zkp proof"),
                    );

                    match result {
                        Ok(result) => {
                            log::debug!(result = ?result, "Applied ZKP proof to the blockchain");
                            // Request epoch ids with our updated state from this peer
                            let future = Self::request_epoch_ids(
                                Arc::clone(&self.blockchain),
                                Arc::clone(&self.network),
                                peer_id,
                            )
                            .boxed();
                            self.epoch_ids_stream.push(future);
                        }
                        Err(result) => {
                            log::debug!(?result, "Failed applying ZKP proof to the blockchain",);

                            // Since it failed applying the ZKP from this peer, we disconnect
                            self.disconnect_peer(peer_id);

                            return Poll::Ready(None);
                        }
                    }
                }
                OutdatedProof { block_height } => {
                    if block_height < self.blockchain.read().block_number() {
                        // The peer is behind us, so we emit it
                        return Poll::Ready(Some(MacroSyncReturn::Outdated(peer_id)));
                    }

                    // If the peer has the same ZKP level as us, then we request epoch ids from this peer
                    let future = Self::request_epoch_ids(
                        Arc::clone(&self.blockchain),
                        Arc::clone(&self.network),
                        peer_id,
                    )
                    .boxed();
                    self.epoch_ids_stream.push(future);

                    return Poll::Pending;
                }
            },
            (Err(zkp_error), peer_id) => {
                // There was an error requesting a proof from this peer, so we disconnect it
                log::debug!(
                    ?zkp_error,
                    %peer_id,
                    "Error requesting zkp from peer",
                );
                self.disconnect_peer(peer_id);
                return Poll::Ready(None);
            }
        }

        Poll::Pending
    }

    fn poll_epoch_ids(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        let epoch_ids = match self.epoch_ids_stream.poll_next_unpin(cx) {
            Poll::Ready(Some(epoch_ids)) => epoch_ids,
            _ => return Poll::Pending,
        };

        if let Some(epoch_ids) = epoch_ids {
            // If the peer didn't find any of our locators, we are done with it and emit it.
            if !epoch_ids.locator_found {
                debug!(
                    peer_id = ?epoch_ids.sender,
                    "Peer is behind or on different chain"
                );
                return Poll::Ready(Some(MacroSyncReturn::Outdated(epoch_ids.sender)));
            } else if epoch_ids.ids.is_empty() && epoch_ids.checkpoint.is_none() {
                // We are synced with this peer.
                debug!(
                    peer_id = ?epoch_ids.sender,
                    "Finished syncing with peer");
                return Poll::Ready(Some(MacroSyncReturn::Good(epoch_ids.sender)));
            }

            // If the macro header process deems a peer useless, it is returned here and we emit it.
            if let Some(agent) = self.request_macro_headers(epoch_ids) {
                return Poll::Ready(Some(MacroSyncReturn::Outdated(agent)));
            }
        }

        Poll::Pending
    }

    fn poll_macro_headers(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some(result)) = self.block_headers.poll_next_unpin(cx) {
            match result {
                (Ok(Some(block)), peer_id) => {
                    let peer_requests = self.peer_requests.get_mut(&peer_id).unwrap();

                    if !peer_requests.update_request(block) {
                        // We received a block we were not expecting from this peer
                        self.disconnect_peer(peer_id);
                        return Poll::Ready(None);
                    }

                    if peer_requests.is_ready() {
                        while let Some((_, block)) = peer_requests.pop_request() {
                            let block = block.expect("At this point the queue should be ready");

                            // Check if the block is still valid for us or if it is outdated before trying to apply it
                            let blockchain = self.blockchain.upgradable_read();

                            let latest_block_number = blockchain.block_number();

                            if block.block_number() < latest_block_number {
                                // The peer is outdated, so we emit it, and we remove it
                                self.peer_requests.remove(&peer_id);
                                return Poll::Ready(Some(MacroSyncReturn::Outdated(peer_id)));
                            }

                            match LightBlockchain::push_macro(blockchain, block.clone()) {
                                Ok(push_result) => {
                                    log::debug!(
                                        block_number = block.block_number(),
                                        ?push_result,
                                        "Pushed a macro block",
                                    );
                                }
                                Err(error) => {
                                    log::debug!(
                                        block_number = block.block_number(),
                                        ?error,
                                        "Failed to push macro block",
                                    );
                                    // We failed applying a block from this peer, so we disconnect it
                                    self.disconnect_peer(peer_id);
                                    return Poll::Ready(None);
                                }
                            }
                        }
                        //  At this point we applied all the pending requests from this peer
                        self.peer_requests.remove(&peer_id);

                        // Re-request epoch ids after applying these blocks in order to know if we are up to date with this peer
                        // or if there is more to sync
                        // Request epoch ids with our updated state from this peer
                        let future = Self::request_epoch_ids(
                            Arc::clone(&self.blockchain),
                            Arc::clone(&self.network),
                            peer_id,
                        )
                        .boxed();
                        self.epoch_ids_stream.push(future);

                        // Pushing the future to FuturesUnordered above does not wake the task that
                        // polls `epoch_ids_stream`. Therefore, we need to wake the task manually.
                        if let Some(waker) = &self.waker {
                            waker.wake_by_ref();
                        }
                    }
                }
                (Ok(None), peer_id) => {
                    trace!("Received a request with None");
                    // If a block request fails, we disconnect from this peer
                    self.disconnect_peer(peer_id);
                }
                (Err(error), peer_id) => {
                    trace!(?error, "Failed block request");
                    // If a block request fails, we disconnect from this peer
                    self.disconnect_peer(peer_id);
                }
            }
        }

        Poll::Pending
    }
}

impl<TNetwork: Network> Stream for LightMacroSync<TNetwork> {
    type Item = MacroSyncReturn<TNetwork::PeerId>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        store_waker!(self, waker, cx);

        if let Poll::Ready(o) = self.poll_network_events(cx) {
            return Poll::Ready(o);
        }

        if let Poll::Ready(o) = self.poll_zkps(cx) {
            return Poll::Ready(o);
        }

        if let Poll::Ready(o) = self.poll_epoch_ids(cx) {
            return Poll::Ready(o);
        }

        if let Poll::Ready(o) = self.poll_macro_headers(cx) {
            return Poll::Ready(o);
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use futures::StreamExt;
    use parking_lot::RwLock;

    use nimiq_block_production::BlockProducer;
    use nimiq_blockchain::{AbstractBlockchain, Blockchain, BlockchainConfig};
    use nimiq_blockchain_proxy::BlockchainProxy;
    use nimiq_database::volatile::VolatileEnvironment;
    use nimiq_light_blockchain::LightBlockchain;
    use nimiq_network_interface::{network::Network, request::request_handler};
    use nimiq_network_mock::{MockHub, MockNetwork};
    use nimiq_primitives::{networks::NetworkId, policy::Policy};
    use nimiq_test_log::test;
    use nimiq_test_utils::blockchain::{produce_macro_blocks_with_txns, signing_key, voting_key};
    use nimiq_utils::time::OffsetTime;

    use crate::messages::{RequestBlock, RequestMacroChain};

    use crate::sync::light::LightMacroSync;
    use crate::sync::syncer::MacroSyncReturn;
    pub const KEYS_PATH: &str = "../.zkp";

    fn blockchain() -> Arc<RwLock<Blockchain>> {
        let time = Arc::new(OffsetTime::new());
        let env = VolatileEnvironment::new(10).unwrap();
        Arc::new(RwLock::new(
            Blockchain::new(
                env,
                BlockchainConfig::default(),
                NetworkId::UnitAlbatross,
                time,
            )
            .unwrap(),
        ))
    }

    fn light_blockchain() -> Arc<RwLock<LightBlockchain>> {
        Arc::new(RwLock::new(LightBlockchain::new(NetworkId::UnitAlbatross)))
    }

    fn spawn_request_handlers<TNetwork: Network>(
        network: &Arc<TNetwork>,
        blockchain: &BlockchainProxy,
    ) {
        tokio::spawn(request_handler(
            network,
            network.receive_requests::<RequestMacroChain>(),
            blockchain,
        ));

        tokio::spawn(request_handler(
            network,
            network.receive_requests::<RequestBlock>(),
            blockchain,
        ));
    }

    #[test(tokio::test)]
    async fn it_terminates_if_the_peer_does_not_answer_the_zkp_request() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let chain = light_blockchain();
        let chain2 = light_blockchain();
        let env = VolatileEnvironment::new(10).unwrap();

        let zkp_component = nimiq_zkp_component::ZKPComponent::new(
            nimiq_blockchain_proxy::BlockchainProxy::Light(Arc::clone(&chain)),
            Arc::clone(&net1),
            false,
            None,
            env,
            PathBuf::from(KEYS_PATH),
        )
        .await;

        let zkp_component_proxy = Arc::new(zkp_component.proxy());

        tokio::spawn(zkp_component);

        let mut sync = LightMacroSync::<MockNetwork>::new(
            Arc::clone(&chain),
            Arc::clone(&net1),
            net1.subscribe_events(),
            zkp_component_proxy,
        );

        spawn_request_handlers(&net2, &BlockchainProxy::from(chain2.clone()));
        net1.dial_mock(&net2);

        match sync.next().await {
            // The peer does not reply to the zkp request, so we disconnect from it and do not emit it
            None => {
                assert_eq!(chain.read().block_number(), 0);
                // Verify the peer was removed
                assert_eq!(sync.peer_requests.len(), 0);
            }
            res => panic!("Unexpected MacroSyncReturn: {:?}", res),
        }
    }

    #[test(tokio::test)]
    async fn it_terminates_if_there_is_nothing_to_sync() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let env = VolatileEnvironment::new(10).unwrap();
        let env2 = VolatileEnvironment::new(10).unwrap();

        let chain1 = light_blockchain();
        let chain2 = blockchain();

        let zkp_component = nimiq_zkp_component::ZKPComponent::new(
            nimiq_blockchain_proxy::BlockchainProxy::Light(Arc::clone(&chain1)),
            Arc::clone(&net1),
            false,
            None,
            env,
            PathBuf::from(KEYS_PATH),
        )
        .await;

        let zkp_component_proxy = Arc::new(zkp_component.proxy());

        tokio::spawn(zkp_component);

        let mut sync = LightMacroSync::<MockNetwork>::new(
            Arc::clone(&chain1),
            Arc::clone(&net1),
            net1.subscribe_events(),
            zkp_component_proxy,
        );

        let zkp_component2 = nimiq_zkp_component::ZKPComponent::new(
            nimiq_blockchain_proxy::BlockchainProxy::Full(Arc::clone(&chain2)),
            Arc::clone(&net2),
            false,
            None,
            env2,
            PathBuf::from(KEYS_PATH),
        )
        .await;

        tokio::spawn(zkp_component2);

        spawn_request_handlers(&net2, &BlockchainProxy::from(chain2.clone()));
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().head(), chain2.read().head());
            }
            res => panic!("Unexpected HistorySyncReturn: {:?}", res),
        }
    }

    // Tries to sync to a full blockchain.
    #[test(tokio::test)]
    async fn it_can_sync_a_single_finalized_epoch() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let env = VolatileEnvironment::new(10).unwrap();
        let env2 = VolatileEnvironment::new(10).unwrap();

        let chain1 = light_blockchain();
        let chain2 = blockchain();

        let producer = BlockProducer::new(signing_key(), voting_key());
        produce_macro_blocks_with_txns(
            &producer,
            &chain2,
            Policy::batches_per_epoch() as usize,
            1,
            0,
        );
        assert_eq!(chain2.read().block_number(), Policy::blocks_per_epoch());

        let zkp_component = nimiq_zkp_component::ZKPComponent::new(
            nimiq_blockchain_proxy::BlockchainProxy::Light(Arc::clone(&chain1)),
            Arc::clone(&net1),
            false,
            None,
            env,
            PathBuf::from(KEYS_PATH),
        )
        .await;

        let zkp_component_proxy = Arc::new(zkp_component.proxy());

        tokio::spawn(zkp_component);

        let mut sync = LightMacroSync::<MockNetwork>::new(
            Arc::clone(&chain1),
            Arc::clone(&net1),
            net1.subscribe_events(),
            zkp_component_proxy,
        );

        let zkp_component2 = nimiq_zkp_component::ZKPComponent::new(
            nimiq_blockchain_proxy::BlockchainProxy::Full(Arc::clone(&chain2)),
            Arc::clone(&net2),
            false,
            None,
            env2,
            PathBuf::from(KEYS_PATH),
        )
        .await;

        tokio::spawn(zkp_component2);

        spawn_request_handlers(&net2, &BlockchainProxy::from(chain2.clone()));
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().head(), chain2.read().head());
            }
            res => panic!("Unexpected HistorySyncReturn: {:?}", res),
        }
    }

    #[test(tokio::test)]
    async fn it_can_sync_a_single_finalized_epoch_and_batch() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let env = VolatileEnvironment::new(10).unwrap();
        let env2 = VolatileEnvironment::new(10).unwrap();

        let chain1 = light_blockchain();
        let chain2 = blockchain();

        let producer = BlockProducer::new(signing_key(), voting_key());
        produce_macro_blocks_with_txns(
            &producer,
            &chain2,
            (Policy::batches_per_epoch() + 1) as usize,
            1,
            0,
        );
        assert_eq!(
            chain2.read().block_number(),
            Policy::blocks_per_epoch() + Policy::blocks_per_batch()
        );

        let zkp_component = nimiq_zkp_component::ZKPComponent::new(
            nimiq_blockchain_proxy::BlockchainProxy::Light(Arc::clone(&chain1)),
            Arc::clone(&net1),
            false,
            None,
            env,
            PathBuf::from(KEYS_PATH),
        )
        .await;

        let zkp_component_proxy = Arc::new(zkp_component.proxy());

        tokio::spawn(zkp_component);

        let mut sync = LightMacroSync::<MockNetwork>::new(
            Arc::clone(&chain1),
            Arc::clone(&net1),
            net1.subscribe_events(),
            zkp_component_proxy,
        );

        let zkp_component2 = nimiq_zkp_component::ZKPComponent::new(
            nimiq_blockchain_proxy::BlockchainProxy::Full(Arc::clone(&chain2)),
            Arc::clone(&net2),
            false,
            None,
            env2,
            PathBuf::from(KEYS_PATH),
        )
        .await;

        tokio::spawn(zkp_component2);

        spawn_request_handlers(&net2, &BlockchainProxy::from(chain2.clone()));
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().head(), chain2.read().head());
            }
            res => panic!("Unexpected HistorySyncReturn: {:?}", res),
        }
    }

    // Tries to sync to a full blockchain.
    #[test(tokio::test)]
    async fn it_can_sync_multiple_batches() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let env = VolatileEnvironment::new(10).unwrap();
        let env2 = VolatileEnvironment::new(10).unwrap();

        let chain1 = light_blockchain();
        let chain2 = blockchain();
        let num_batches = (Policy::batches_per_epoch() - 1) as usize;
        let producer = BlockProducer::new(signing_key(), voting_key());
        produce_macro_blocks_with_txns(&producer, &chain2, num_batches, 1, 0);
        assert_eq!(
            chain2.read().block_number(),
            num_batches as u32 * Policy::blocks_per_batch()
        );

        let zkp_component = nimiq_zkp_component::ZKPComponent::new(
            nimiq_blockchain_proxy::BlockchainProxy::Light(Arc::clone(&chain1)),
            Arc::clone(&net1),
            false,
            None,
            env,
            PathBuf::from(KEYS_PATH),
        )
        .await;

        let zkp_component_proxy = Arc::new(zkp_component.proxy());

        tokio::spawn(zkp_component);

        let mut sync = LightMacroSync::<MockNetwork>::new(
            Arc::clone(&chain1),
            Arc::clone(&net1),
            net1.subscribe_events(),
            zkp_component_proxy,
        );

        let zkp_component2 = nimiq_zkp_component::ZKPComponent::new(
            nimiq_blockchain_proxy::BlockchainProxy::Full(Arc::clone(&chain2)),
            Arc::clone(&net2),
            false,
            None,
            env2,
            PathBuf::from(KEYS_PATH),
        )
        .await;

        tokio::spawn(zkp_component2);

        spawn_request_handlers(&net2, &BlockchainProxy::from(chain2.clone()));
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().head(), chain2.read().head());
            }
            res => panic!("Unexpected HistorySyncReturn: {:?}", res),
        }
    }
}
