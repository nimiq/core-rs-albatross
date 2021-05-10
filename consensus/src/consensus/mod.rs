use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use futures::stream::BoxStream;
use futures::task::{Context, Poll};
use futures::{FutureExt, Stream, StreamExt};
use tokio::sync::broadcast::{
    channel as broadcast, Receiver as BroadcastReceiver, Sender as BroadcastSender,
};

use block::Block;
use blockchain::Blockchain;
use database::Environment;
use mempool::{Mempool, ReturnCode};
use network_interface::network::Network;
use nimiq_network_interface::network::Topic;
use transaction::Transaction;

use crate::consensus::head_requests::{HeadRequests, HeadRequestsResult};
use crate::consensus_agent::ConsensusAgent;
use crate::sync::block_queue::{BlockQueue, BlockQueueConfig, BlockQueueEvent, BlockTopic};
use crate::sync::request_component::BlockRequestComponent;

mod head_requests;
mod request_response;

#[derive(Clone, Debug, Default)]
pub struct TransactionTopic;

impl Topic for TransactionTopic {
    type Item = Transaction;

    fn topic(&self) -> String {
        "transactions".to_owned()
    }

    fn validate(&self) -> bool {
        false
    }
}

pub struct ConsensusProxy<N: Network> {
    pub blockchain: Arc<Blockchain>,
    pub network: Arc<N>,
    pub mempool: Arc<Mempool>,
    established_flag: Arc<AtomicBool>,
}

impl<N: Network> Clone for ConsensusProxy<N> {
    fn clone(&self) -> Self {
        Self {
            blockchain: Arc::clone(&self.blockchain),
            network: Arc::clone(&self.network),
            mempool: Arc::clone(&self.mempool),
            established_flag: Arc::clone(&self.established_flag),
        }
    }
}

impl<N: Network> ConsensusProxy<N> {
    pub async fn send_transaction(&self, tx: Transaction) -> Result<ReturnCode, N::Error> {
        match self.mempool.push_transaction(tx.clone()) {
            ReturnCode::Accepted => {}
            e => return Ok(e),
        }

        self.network
            .publish(&TransactionTopic::default(), tx)
            .await
            .map(|_| ReturnCode::Accepted)
    }

    pub fn is_established(&self) -> bool {
        self.established_flag.load(Ordering::Acquire)
    }
}

pub enum ConsensusEvent<N: Network> {
    PeerMacroSynced(Weak<ConsensusAgent<N::PeerType>>),
    PeerLeft,
    Established,
    Lost,
}

impl<N: Network> Clone for ConsensusEvent<N> {
    fn clone(&self) -> Self {
        match self {
            ConsensusEvent::PeerMacroSynced(peer) => ConsensusEvent::PeerMacroSynced(peer.clone()),
            ConsensusEvent::Established => ConsensusEvent::Established,
            ConsensusEvent::Lost => ConsensusEvent::Lost,
            ConsensusEvent::PeerLeft => ConsensusEvent::PeerLeft,
        }
    }
}

pub struct Consensus<N: Network> {
    pub blockchain: Arc<Blockchain>,
    pub mempool: Arc<Mempool>,
    pub network: Arc<N>,
    pub env: Environment,

    block_queue: BlockQueue<N::PeerType, BlockRequestComponent<N::PeerType>>,
    tx_stream: BoxStream<'static, Transaction>,

    events: BroadcastSender<ConsensusEvent<N>>,
    established_flag: Arc<AtomicBool>,
    head_requests: Option<HeadRequests<N::PeerType>>,
    head_requests_time: Option<Instant>,

    min_peers: usize,
}

impl<N: Network> Consensus<N> {
    /// Minimum number of peers for consensus to be established.
    const MIN_PEERS_ESTABLISHED: usize = 3;
    /// Minimum number of block announcements extending the chain for consensus to be established.
    const MIN_BLOCKS_ESTABLISHED: usize = 5;
    /// Timeout after which head requests will be performed again to determine consensus established state.
    const HEAD_REQUESTS_TIMEOUT: Duration = Duration::from_secs(20); // currently 2 * view change delay

    pub async fn from_network(
        env: Environment,
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        network: Arc<N>,
        sync_protocol: BoxStream<'static, Arc<ConsensusAgent<N::PeerType>>>,
    ) -> Self {
        Self::with_min_peers(
            env,
            blockchain,
            mempool,
            network,
            sync_protocol,
            Self::MIN_PEERS_ESTABLISHED,
        )
        .await
    }

    pub async fn with_min_peers(
        env: Environment,
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        network: Arc<N>,
        sync_protocol: BoxStream<'static, Arc<ConsensusAgent<N::PeerType>>>,
        min_peers: usize,
    ) -> Self {
        let block_stream = network
            .subscribe::<BlockTopic>(&BlockTopic::default())
            .await
            .unwrap()
            .map(|(block, _peer_id)| block)
            .boxed();

        let tx_stream = network
            .subscribe::<TransactionTopic>(&TransactionTopic::default())
            .await
            .unwrap()
            .map(|(tx, _peer_id)| tx)
            .boxed();

        Self::new(
            env,
            blockchain,
            mempool,
            network,
            block_stream,
            tx_stream,
            sync_protocol,
            min_peers,
        )
    }

    pub fn new(
        env: Environment,
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        network: Arc<N>,
        block_stream: BoxStream<'static, Block>,
        tx_stream: BoxStream<'static, Transaction>,
        sync_protocol: BoxStream<'static, Arc<ConsensusAgent<N::PeerType>>>,
        min_peers: usize,
    ) -> Self {
        let (tx, _rx) = broadcast(256);

        let request_component =
            BlockRequestComponent::new(sync_protocol, network.subscribe_events());

        let block_queue = BlockQueue::new(
            BlockQueueConfig::default(),
            Arc::clone(&blockchain),
            request_component,
            block_stream,
        );

        Self::init_network_requests(&network, &blockchain);

        Consensus {
            blockchain,
            mempool,
            network,
            env,

            block_queue,
            tx_stream,
            events: tx,

            established_flag: Arc::new(AtomicBool::new(false)),
            head_requests: None,
            head_requests_time: None,

            min_peers,
        }
    }

    pub fn subscribe_events(&self) -> BroadcastReceiver<ConsensusEvent<N>> {
        self.events.subscribe()
    }

    pub fn is_established(&self) -> bool {
        self.established_flag.load(Ordering::Acquire)
    }

    pub fn num_agents(&self) -> usize {
        self.block_queue.num_peers()
    }

    pub fn proxy(&self) -> ConsensusProxy<N> {
        ConsensusProxy {
            blockchain: Arc::clone(&self.blockchain),
            network: Arc::clone(&self.network),
            mempool: Arc::clone(&self.mempool),
            established_flag: Arc::clone(&self.established_flag),
        }
    }

    /// Forcefully sets consensus established, should be used for tests only.
    pub fn force_established(&mut self) {
        trace!("Consensus forcefully established.");
        self.established_flag.swap(true, Ordering::Release);

        // Also stop any other checks.
        self.head_requests = None;
        self.head_requests_time = None;
        self.events.send(ConsensusEvent::Established).ok();
    }

    /// Calculates and sets established state, returns a ConsensusEvent if the state changed.
    /// Once consensus is established, we can only loose it if we loose all our peers.
    /// To reach consensus established state, we need at least `minPeers` peers and
    /// one of the following conditions must be true:
    /// - we accepted at least `MIN_BLOCKS_ESTABLISHED` block announcements
    /// - we know at least 2/3 of the head blocks of our peers
    ///
    /// The latter check is started immediately once we reach the minimum number of peers
    /// and is potentially repeated in an interval of `HEAD_REQUESTS_TIMEOUT` until one
    /// of the conditions above is true.
    /// Any unknown blocks resulting of the head check are handled similarly as block announcements
    /// via the block queue.
    fn set_established(
        &mut self,
        finished_head_request: Option<HeadRequestsResult>,
    ) -> Option<ConsensusEvent<N>> {
        // We can only loose established state right now if we loose all our peers.
        if self.is_established() {
            if self.num_agents() == 0 {
                warn!("Lost consensus!");
                self.established_flag.swap(false, Ordering::Release);
                return Some(ConsensusEvent::Lost);
            }
        } else {
            // We have two conditions on whether we move to the established state.
            // First, we always need a minimum number of peers connected.
            // Then, we check that we either:
            // - accepted a minimum number of block announcements, or
            // - know the head state of a majority of our peers
            if self.num_agents() >= self.min_peers {
                trace!("Trying to establish consensus, number of synced peers satisfied.");
                if self.block_queue.accepted_block_announcements() >= Self::MIN_BLOCKS_ESTABLISHED {
                    info!("Consensus established, number of accepted announcements satisfied.");
                    self.established_flag.swap(true, Ordering::Release);

                    // Also stop any other checks.
                    self.head_requests = None;
                    self.head_requests_time = None;
                    return Some(ConsensusEvent::Established);
                } else {
                    // The head state check is carried out immediately after we reach the minimum
                    // number of peers and then after certain time intervals until consensus is reached.
                    // If we have a finished one, check its outcome.
                    if let Some(head_request) = finished_head_request {
                        trace!("Trying to establish consensus, checking head request ({} known, {} unknown).", head_request.num_known_blocks, head_request.num_unknown_blocks);
                        // We would like that 2/3 of our peers have a known state.
                        if head_request.num_known_blocks > 2 * head_request.num_unknown_blocks {
                            info!("Consensus established, 2/3 of heads known.");
                            self.established_flag.swap(true, Ordering::Release);
                            return Some(ConsensusEvent::Established);
                        }
                    }
                    // If there's no ongoing head request, check whether we should start a new one.
                    if self.head_requests.is_none() {
                        // This is the case if `head_requests_time` is unset or the timeout is hit.
                        let should_start_request = self
                            .head_requests_time
                            .map(|time| time.elapsed() >= Self::HEAD_REQUESTS_TIMEOUT)
                            .unwrap_or(true);
                        if should_start_request {
                            trace!("Trying to establish consensus, initiating head requests.");
                            self.head_requests = Some(HeadRequests::new(
                                self.block_queue.peers(),
                                Arc::clone(&self.blockchain),
                            ));
                            self.head_requests_time = Some(Instant::now());
                        }
                    }
                }
            }
        }
        None
    }
}

impl<N: Network> Stream for Consensus<N> {
    type Item = ConsensusEvent<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        macro_rules! return_event {
            ($event:expr) => {
                self.events.send($event.clone()).ok(); // Ignore result.
                return Poll::Ready(Some($event));
            };
        }

        // 1. Poll and advance block queue
        while let Poll::Ready(Some(event)) = self.block_queue.poll_next_unpin(cx) {
            match event {
                BlockQueueEvent::PeerMacroSynced(peer) => {
                    let e = ConsensusEvent::PeerMacroSynced(peer);
                    return_event!(e);
                }
                BlockQueueEvent::PeerLeft(_) => {
                    return_event!(ConsensusEvent::PeerLeft);
                }
                _ => {}
            }
        }

        // Check consensus established state on changes.
        if let Some(event) = self.set_established(None) {
            return_event!(event);
        }

        // 2. Poll and push transactions once consensus is established.
        if self.is_established() {
            while let Poll::Ready(Some(tx)) = self.tx_stream.poll_next_unpin(cx) {
                // TODO: React on result.
                self.mempool.push_transaction(tx);
            }
        }

        // 3. Poll any head requests if active.
        if let Some(ref mut head_requests) = self.head_requests {
            if let Poll::Ready(mut result) = head_requests.poll_unpin(cx) {
                // Push unknown blocks to the block queue, trying to sync.
                for block in result.unknown_blocks.drain(..) {
                    self.block_queue.push_block(block);
                }
                // Update established state using the result.
                if let Some(event) = self.set_established(Some(result)) {
                    return_event!(event);
                }
            }
        }

        Poll::Pending
    }
}
