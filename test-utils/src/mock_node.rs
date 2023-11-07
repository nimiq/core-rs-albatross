use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::{future::BoxFuture, ready, stream::BoxStream, FutureExt, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::{
    messages::*,
    sync::live::{diff_queue::RequestTrieDiff, state_queue::RequestChunk},
};
use nimiq_database::volatile::VolatileDatabase;
use nimiq_network_interface::{
    network::Network as NetworkInterface,
    request::{Handle, Request},
};
use nimiq_network_mock::MockHub;
use nimiq_primitives::{networks::NetworkId, trie::TrieItem};
use nimiq_utils::time::OffsetTime;
use parking_lot::RwLock;

use crate::test_network::TestNetwork;

pub struct MockHandler<
    N: NetworkInterface + TestNetwork,
    Req: Request + Handle<N, Req::Response, T>,
    T: Send + Sync + Unpin,
> {
    network: Arc<N>,
    subscription: BoxStream<'static, (Req, N::RequestId, N::PeerId)>,
    paused: bool,
    pause_waker: Option<Waker>,
    request_handler: Option<fn(N::PeerId, &Req, &T) -> Req::Response>,
    response_future: Option<BoxFuture<'static, u16>>,
    environment: T,
}

impl<
        N: NetworkInterface + TestNetwork,
        Req: Request + Handle<N, Req::Response, T>,
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
            paused: false,
            pause_waker: None,
            request_handler: None,
            response_future: None,
            environment,
        }
    }
    pub fn pause(&mut self) {
        self.paused = true;
    }
    pub fn unpause(&mut self) {
        if let Some(waker) = self.pause_waker.take() {
            waker.wake();
        }
        self.paused = false;
    }
    pub fn set(&mut self, request_handler: fn(N::PeerId, &Req, &T) -> Req::Response) {
        self.request_handler = Some(request_handler);
    }
    pub fn unset(&mut self) {
        self.request_handler = None;
    }
}

impl<
        N: NetworkInterface + TestNetwork,
        Req: Request + Handle<N, Req::Response, T>,
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

            if self.paused {
                self.pause_waker = Some(cx.waker().clone());
                return Poll::Pending;
            }

            if let Some((request, request_id, peer_id)) =
                ready!(self.subscription.poll_next_unpin(cx))
            {
                log::info!(
                    "Received request {:?}, has custom handler: {}",
                    request,
                    self.request_handler.is_some()
                );
                let response = if let Some(ref h) = self.request_handler {
                    h(peer_id, &request, &self.environment)
                } else {
                    // call original handler with blockchain
                    request.handle(peer_id, &self.environment)
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
    pub request_partial_diff_handler: MockHandler<N, RequestTrieDiff, Arc<RwLock<Blockchain>>>,
    pub request_chunk_handler: MockHandler<N, RequestChunk, Arc<RwLock<Blockchain>>>,
}

impl<N: NetworkInterface + TestNetwork> MockNode<N> {
    pub async fn new(
        peer_id: u64,
        block: Block,
        accounts: Vec<TrieItem>,
        hub: &mut Option<MockHub>,
    ) -> Self {
        let network = N::build_network(peer_id, block.hash(), hub).await;
        let env = VolatileDatabase::new(20).unwrap();
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
        let partial_diff_subscription = network.receive_requests::<RequestTrieDiff>();
        let chunk_subscription = network.receive_requests::<RequestChunk>();

        let request_missing_block_handler = MockHandler::new(
            Arc::clone(&network),
            missing_block_subscription,
            BlockchainProxy::Full(Arc::clone(&blockchain)),
        );
        let request_partial_diff_handler = MockHandler::new(
            Arc::clone(&network),
            partial_diff_subscription,
            Arc::clone(&blockchain),
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
            request_partial_diff_handler,
            request_chunk_handler,
        }
    }
}

impl<N: NetworkInterface + TestNetwork> Stream for MockNode<N> {
    type Item = u16;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Fetching and processing block requests
        if let Poll::Ready(Some(request)) = self.request_missing_block_handler.poll_next_unpin(cx) {
            return Poll::Ready(Some(request));
        }

        if let Poll::Ready(Some(request)) = self.request_partial_diff_handler.poll_next_unpin(cx) {
            return Poll::Ready(Some(request));
        }

        if let Poll::Ready(Some(request)) = self.request_chunk_handler.poll_next_unpin(cx) {
            return Poll::Ready(Some(request));
        }

        Poll::Pending
    }
}
