use std::pin::Pin;
use std::task::Context;
use std::{sync::Arc, task::Poll};

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{ready, FutureExt, Stream, StreamExt};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;
use parking_lot::RwLock;

use nimiq_account::Account;
use nimiq_block::Block;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain::BlockchainConfig;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::messages::*;
use nimiq_consensus::sync::live::state_queue::{RequestChunk, ResponseChunk};
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_interface::request::{Handle, Request};
use nimiq_network_mock::MockHub;
use nimiq_trie::key_nibbles::KeyNibbles;

use crate::test_network::TestNetwork;

pub struct MockHandler<
    N: NetworkInterface + TestNetwork,
    Req: Request + Handle<Req::Response, T>,
    T: Send + Sync + Unpin,
> {
    network: Arc<N>,
    subscription: BoxStream<'static, (Req, N::RequestId, N::PeerId)>,
    request_handler: Option<fn(&Req, &T) -> Req::Response>,
    response_future: Option<BoxFuture<'static, u16>>,
    environment: T,
}

impl<
        N: NetworkInterface + TestNetwork,
        Req: Request + Handle<Req::Response, T>,
        T: Send + Sync + Unpin,
    > MockHandler<N, Req, T>
{
    pub fn new(
        network: Arc<N>,
        subscription: BoxStream<'static, (Req, N::RequestId, N::PeerId)>,
        environment: T,
    ) -> Self {
        Self {
            network,
            subscription,
            request_handler: None,
            response_future: None,
            environment,
        }
    }
    pub fn set_handler(&mut self, request_handler: Option<fn(&Req, &T) -> Req::Response>) {
        self.request_handler = request_handler;
    }
}

impl<
        N: NetworkInterface + TestNetwork,
        Req: Request + Handle<Req::Response, T>,
        T: Send + Sync + Unpin,
    > Stream for MockHandler<N, Req, T>
{
    type Item = u16;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(ref mut response_future) = self.response_future {
                let req = ready!(response_future.poll_unpin(cx));
                self.response_future = None;
                return Poll::Ready(Some(req));
            }

            if let Some((request, request_id, _peer_id)) =
                ready!(self.subscription.poll_next_unpin(cx))
            {
                log::info!(
                    "Received request {:?}, has custom handler: {}",
                    request,
                    self.request_handler.is_some()
                );
                let response = if let Some(ref h) = self.request_handler {
                    h(&request, &self.environment)
                } else {
                    // call original handler with blockchain
                    request.handle(&self.environment)
                };

                let network2 = Arc::clone(&self.network);
                let send_future = async move {
                    network2
                        .respond::<Req>(request_id, response)
                        .map(move |e| {
                            if let Err(e) = e {
                                log::debug!(
                                    "[{:?}] Failed to send {} response: {:?}",
                                    request_id,
                                    std::any::type_name::<Req>(),
                                    e
                                );
                            }
                        })
                        .await;
                    Req::TYPE_ID
                }
                .boxed();
                self.response_future = Some(send_future);
            } else {
                return Poll::Ready(None);
            }
        }
    }
}
pub struct MockNode<N: NetworkInterface + TestNetwork> {
    pub network: Arc<N>,
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub request_missing_block_handler: MockHandler<N, RequestMissingBlocks, BlockchainProxy>,
    pub request_chunk_handler: MockHandler<N, RequestChunk, Arc<RwLock<Blockchain>>>,
}

impl<N: NetworkInterface + TestNetwork> MockNode<N> {
    pub async fn new(
        peer_id: u64,
        block: Block,
        accounts: Vec<(KeyNibbles, Account)>,
        hub: &mut Option<MockHub>,
    ) -> Self {
        let network = N::build_network(peer_id, block.hash(), hub).await;
        let env = VolatileEnvironment::new(14).unwrap();
        let clock = Arc::new(OffsetTime::new());

        let blockchain = Arc::new(RwLock::new(
            Blockchain::with_genesis(
                env,
                BlockchainConfig::default(),
                Arc::clone(&clock),
                NetworkId::UnitAlbatross,
                block,
                accounts,
            )
            .unwrap(),
        ));
        Self::with_network_and_blockchain(network, blockchain)
    }

    pub fn with_network_and_blockchain(
        network: Arc<N>,
        blockchain: Arc<RwLock<Blockchain>>,
    ) -> Self {
        let missing_block_subscription = network.receive_requests::<RequestMissingBlocks>();
        let chunk_subscription = network.receive_requests::<RequestChunk>();

        let request_missing_block_handler = MockHandler::new(
            Arc::clone(&network),
            missing_block_subscription,
            BlockchainProxy::Full(Arc::clone(&blockchain)),
        );
        let request_chunk_handler = MockHandler::new(
            Arc::clone(&network),
            chunk_subscription,
            Arc::clone(&blockchain),
        );

        Self {
            network,
            blockchain,
            request_missing_block_handler,
            request_chunk_handler,
        }
    }

    pub fn set_missing_block_handler(
        &mut self,
        request_missing_block_handler: Option<
            fn(&RequestMissingBlocks, &BlockchainProxy) -> ResponseBlocks,
        >,
    ) {
        self.request_missing_block_handler
            .set_handler(request_missing_block_handler);
    }

    pub fn set_chunk_handler(
        &mut self,
        request_chunk_handler: Option<fn(&RequestChunk, &Arc<RwLock<Blockchain>>) -> ResponseChunk>,
    ) {
        self.request_chunk_handler
            .set_handler(request_chunk_handler);
    }
}

impl<N: NetworkInterface + TestNetwork> Stream for MockNode<N> {
    type Item = u16;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Fetching and processing block requests
        if let Poll::Ready(Some(request)) = self.request_missing_block_handler.poll_next_unpin(cx) {
            return Poll::Ready(Some(request));
        }

        if let Poll::Ready(Some(request)) = self.request_chunk_handler.poll_next_unpin(cx) {
            return Poll::Ready(Some(request));
        }

        Poll::Pending
    }
}
