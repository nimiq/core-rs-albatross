use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use futures::{stream::BoxStream, StreamExt};
use parking_lot::RwLock;

use nimiq_primitives::policy;

use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_network_interface::{network::Network, request::Request};

use crate::messages::handlers::Handle;
use crate::messages::{
    RequestBatchSet, RequestBlock, RequestHead, RequestHistoryChunk, RequestMacroChain,
    RequestMissingBlocks,
};
use crate::Consensus;

use super::RateLimit;

impl<N: Network> Consensus<N> {
    const MAX_CONCURRENT_HANDLERS: usize = 64;

    pub(super) fn init_network_request_receivers(
        network: &Arc<N>,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) {
        let stream = network.receive_requests::<RequestMacroChain>();
        tokio::spawn(Self::request_handler(
            network,
            stream,
            blockchain,
            policy::MAX_RESPONSE_REQUEST_MACRO_CHAIN,
            policy::MAX_RESPONSE_REQUEST_BLOCK_RANGE,
        ));

        let stream = network.receive_requests::<RequestBatchSet>();
        tokio::spawn(Self::request_handler(
            network,
            stream,
            blockchain,
            policy::MAX_RESPONSE_REQUEST_BATCH_SET,
            policy::MAX_RESPONSE_REQUEST_BLOCK_RANGE,
        ));

        let stream = network.receive_requests::<RequestHistoryChunk>();
        tokio::spawn(Self::request_handler(
            network,
            stream,
            blockchain,
            policy::MAX_RESPONSE_REQUEST_HISTORY_CHUNK,
            policy::MAX_RESPONSE_REQUEST_BLOCK_RANGE,
        ));

        let stream = network.receive_requests::<RequestBlock>();
        tokio::spawn(Self::request_handler(
            network,
            stream,
            blockchain,
            policy::MAX_RESPONSE_REQUEST_BLOCK,
            policy::MAX_RESPONSE_REQUEST_BLOCK_RANGE,
        ));

        let stream = network.receive_requests::<RequestMissingBlocks>();
        tokio::spawn(Self::request_handler(
            network,
            stream,
            blockchain,
            policy::MAX_RESPONSE_REQUEST_MISSING_BLOCKS,
            policy::MAX_RESPONSE_REQUEST_BLOCK_RANGE,
        ));

        let stream = network.receive_requests::<RequestHead>();
        tokio::spawn(Self::request_handler(
            network,
            stream,
            blockchain,
            policy::MAX_RESPONSE_REQUEST_HEAD,
            policy::MAX_RESPONSE_REQUEST_BLOCK_RANGE,
        ));
    }

    pub(crate) fn request_handler<Req: Handle<Req::Response, N> + Request>(
        network: &Arc<N>,
        stream: BoxStream<'static, (Req, N::RequestId, N::PeerId)>,
        blockchain: &Arc<RwLock<Blockchain>>,
        max_requests: u16,
        block_range: u32,
    ) -> impl Future<Output = ()> {
        let blockchain = Arc::clone(blockchain);
        let network = Arc::clone(network);
        let peers_request_limit = Arc::new(futures::lock::Mutex::new(HashMap::with_capacity(
            network.get_peers().len() * 5,
        )));

        async move {
            stream
                .for_each_concurrent(
                    Self::MAX_CONCURRENT_HANDLERS,
                    |(msg, request_id, peer_id)| {
                        let network = Arc::clone(&network);
                        let blockchain = Arc::clone(&blockchain);
                        let peers_request_limit = Arc::clone(&peers_request_limit);
                        async move {
                            tokio::spawn(async move {
                               trace!("[{:?}] {:?} {:#?}", request_id, peer_id, msg);

                                // Gets lock of peer requests limits read and write on it
                                let mut peers_request_limit = peers_request_limit.lock().await;
                                // If the peer has never sent a request of this type, creates a new entry
                                let requests_limit = peers_request_limit
                                    .entry(peer_id)
                                    .or_insert_with(|| RateLimit::new(max_requests, block_range));

                                // Ensures that the request is allowed based on the set limits and updates the counter.
                                // Returns early if not allowed.
                                if !requests_limit
                                    .increment_and_is_allowed(1, blockchain.read().block_number())
                                {
                                    log::debug!(
                                        "[{:?}][{:?}] {:?} Exceeded max requests rate {:?}/{:?} blocks",
                                        request_id,
                                        peer_id,
                                        std::any::type_name::<Req>(),
                                        max_requests,
                                        block_range
                                    );
                                    return;
                                }
                                // There is no further changes or reads to the peer requests counters, so we drop our.
                                std::mem::drop(peers_request_limit);

                                // Try to send the response, logging to debug if it fails
                                if let Err(err) = network
                                    .respond::<Req>(request_id, msg.handle(&blockchain))
                                    .await
                                {
                                    log::debug!(
                                        "[{:?}] Failed to send {} response: {:?}",
                                        request_id,
                                        std::any::type_name::<Req>(),
                                        err
                                    );
                                };
                            })
                            .await
                            .expect("Request handler panicked")
                        }
                    },
                )
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nimiq_network_interface::request::{InboundRequestError, RequestError};
    use parking_lot::RwLock;

    use nimiq_blockchain::Blockchain;
    use nimiq_database::volatile::VolatileEnvironment;
    use nimiq_network_interface::network::Network;
    use nimiq_network_mock::MockHub;
    use nimiq_primitives::networks::NetworkId;
    use nimiq_primitives::policy;
    use nimiq_test_log::test;
    use nimiq_utils::time::OffsetTime;

    use crate::messages::RequestHead;
    use crate::Consensus;

    fn blockchain() -> Arc<RwLock<Blockchain>> {
        let time = Arc::new(OffsetTime::new());
        let env = VolatileEnvironment::new(10).unwrap();
        Arc::new(RwLock::new(
            Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
        ))
    }

    fn spawn_head_request_handler<TNetwork: Network>(
        network: &Arc<TNetwork>,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) {
        tokio::spawn(Consensus::<TNetwork>::request_handler(
            network,
            network.receive_requests::<RequestHead>(),
            blockchain,
            1,
            policy::MAX_RESPONSE_REQUEST_BLOCK_RANGE,
        ));
    }

    #[test(tokio::test)]
    async fn it_can_limit_requests_rate() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let chain1 = blockchain();

        assert!(net2.dial_peer(net1.peer_id()).await.is_ok());
        spawn_head_request_handler(&net1, &chain1);

        // The first head request is sent and this should be the only request that gets an Ok response.
        assert!(net2.has_peer(net1.peer_id()));
        let response = net2
            .request::<RequestHead>(RequestHead {}, net1.peer_id())
            .await;

        match response {
            Ok(_) => {}
            Err(e) => {
                panic!(
                    "First request should be the only one working, however it failed - {:?}",
                    e
                )
            }
        }

        // Rate limit was exceeded, from now on it should fail with a timeout.
        for _ in 0..4 {
            assert!(net2.has_peer(net1.peer_id()));
            let response = net2
                .request::<RequestHead>(RequestHead {}, net1.peer_id())
                .await;

            match response {
                Err(RequestError::InboundRequest(InboundRequestError::Timeout)) => {}
                Ok(_) => {
                    panic!("Expected to timeout due to exceeding the max rate limit, returned Ok instead.");
                }
                Err(e) => {
                    panic!("Expected to timeout due to exceeding the max rate limit, failed with a different error message - {:?}", e);
                }
            }
        }
    }
}
