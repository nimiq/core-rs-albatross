use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::task::{Context, Poll};
use futures::{future::BoxFuture, stream::BoxStream, Future, FutureExt, StreamExt};
use tokio::sync::broadcast::{channel as broadcast, Sender as BroadcastSender};
use tokio::time::Sleep;
use tokio_stream::wrappers::BroadcastStream;

use blockchain::{AbstractBlockchain, Blockchain};
use database::Environment;
use mempool::{Mempool, ReturnCode};
use network_interface::network::{MsgAcceptance, Network, Topic};
use transaction::Transaction;

use crate::consensus::head_requests::{HeadRequests, HeadRequestsResult};
use crate::sync::block_queue::{BlockQueue, BlockQueueConfig, BlockQueueEvent};
use crate::sync::request_component::{BlockRequestComponent, HistorySyncStream};

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
        true
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

#[derive(Clone)]
pub enum ConsensusEvent {
    Established,
    Lost,
}

pub struct Consensus<N: Network> {
    pub blockchain: Arc<Blockchain>,
    pub mempool: Arc<Mempool>,
    pub network: Arc<N>,
    pub env: Environment,

    block_queue: BlockQueue<N, BlockRequestComponent<N::PeerType>>,
    tx_future: BoxFuture<'static, ()>,

    /// A Delay which exists purely for the waker on its poll to reactivate the task running Consensus::poll
    next_execution_timer: Option<Pin<Box<Sleep>>>,

    events: BroadcastSender<ConsensusEvent>,
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
    const HEAD_REQUESTS_TIMEOUT: Duration = Duration::from_secs(10);

    /// Timeout after which the consensus is polled after it ran last
    ///
    /// TODO: Set appropriate duration
    const CONSENSUS_POLL_TIMER: Duration = Duration::from_secs(1);

    pub async fn from_network(
        env: Environment,
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        network: Arc<N>,
        sync_protocol: Pin<Box<dyn HistorySyncStream<N::PeerType>>>,
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
        sync_protocol: Pin<Box<dyn HistorySyncStream<N::PeerType>>>,
        min_peers: usize,
    ) -> Self {
        let request_component =
            BlockRequestComponent::new(sync_protocol, network.subscribe_events());

        let block_queue = BlockQueue::new(
            BlockQueueConfig::default(),
            Arc::clone(&blockchain),
            Arc::clone(&network),
            request_component,
        )
        .await;

        let tx_stream = network
            .subscribe::<TransactionTopic>(&TransactionTopic::default())
            .await
            .unwrap()
            .boxed();

        Self::new(
            env,
            blockchain,
            mempool,
            network,
            block_queue,
            tx_stream,
            min_peers,
        )
    }

    pub fn new(
        env: Environment,
        blockchain: Arc<Blockchain>,
        mempool: Arc<Mempool>,
        network: Arc<N>,
        block_queue: BlockQueue<N, BlockRequestComponent<N::PeerType>>,
        tx_stream: BoxStream<'static, (Transaction, <N as Network>::PubsubId)>,
        min_peers: usize,
    ) -> Self {
        let (tx, _rx) = broadcast(256);

        Self::init_network_requests(&network, &blockchain);

        let established_flag = Arc::new(AtomicBool::new(false));

        let established_flag1 = Arc::clone(&established_flag);
        let mempool1 = Arc::clone(&mempool);
        let network1 = Arc::clone(&network);
        let tx_future = async move {
            tx_stream
                .for_each(|(tx, pubsub_id)| async {

                    // Make sure consensus has been established before processing transactions
                    if established_flag1.load(Ordering::Acquire) {
                        let acceptance = match mempool1.push_transaction(tx) {
                            ReturnCode::Accepted | ReturnCode::Known => MsgAcceptance::Accept,
                            ReturnCode::Filtered | ReturnCode::FeeTooLow => MsgAcceptance::Ignore,
                            ReturnCode::Invalid => MsgAcceptance::Reject,
                        };

                        // Let the network layer know if it should relay the message this tx came from
                        match network1.validate_message(pubsub_id, acceptance).await {
                            Ok(true) => trace!("The tx message was relayed succesfully"),
                            Ok(false) => warn!("Validation took too long: the tx message was no longer in the message cache"),
                            Err(e) => error!("Network error while relaying tx message: {}", e),
                        };
                    }
                })
                .await
        }
        .boxed();

        let timer = Box::pin(tokio::time::sleep(Self::CONSENSUS_POLL_TIMER));

        Consensus {
            blockchain,
            mempool,
            network,
            env,
            block_queue,
            tx_future,
            events: tx,
            next_execution_timer: Some(timer),
            established_flag,
            head_requests: None,
            head_requests_time: None,

            min_peers,
        }
    }

    pub fn subscribe_events(&self) -> BroadcastStream<ConsensusEvent> {
        BroadcastStream::new(self.events.subscribe())
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
        finished_head_request: Option<HeadRequestsResult<N::PeerType>>,
    ) -> Option<ConsensusEvent> {
        // We can only lose established state right now if we drop below our minimum peer threshold.
        if self.is_established() {
            if self.num_agents() < self.min_peers {
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
                        debug!("Trying to establish consensus, checking head request ({} known, {} unknown).", head_request.num_known_blocks, head_request.num_unknown_blocks);
                        // We would like that 2/3 of our peers have a known state.
                        if head_request.num_known_blocks >= 2 * head_request.num_unknown_blocks {
                            info!("Consensus established, 2/3 of heads known.");
                            self.established_flag.swap(true, Ordering::Release);
                            return Some(ConsensusEvent::Established);
                        }
                    }
                    // If there's no ongoing head request, check whether we should start a new one.
                    self.request_heads();
                }
            }
        }
        None
    }

    /// Requests heads from connected peers in a predefined interval.
    fn request_heads(&mut self) {
        // If we do not have consensus, there's no ongoing head request,
        // and we have at least one peer, check whether we should start a new one.
        if !self.is_established()
            && self.head_requests.is_none()
            && (self.num_agents() > 0 || self.min_peers == 0)
        {
            // This is the case if `head_requests_time` is unset or the timeout is hit.
            let should_start_request = self
                .head_requests_time
                .map(|time| time.elapsed() >= Self::HEAD_REQUESTS_TIMEOUT)
                .unwrap_or(true);
            if should_start_request {
                debug!("Trying to establish consensus, initiating head requests.");
                self.head_requests = Some(HeadRequests::new(
                    self.block_queue.peers(),
                    Arc::clone(&self.blockchain),
                ));
                self.head_requests_time = Some(Instant::now());
            }
        }
    }
}

impl<N: Network> Future for Consensus<N> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // 1. Poll and advance block queue
        while let Poll::Ready(Some(event)) = self.block_queue.poll_next_unpin(cx) {
            match event {
                BlockQueueEvent::AcceptedAnnouncedBlock(_) => {
                    debug!("Now at block #{}", self.blockchain.block_number());
                }
                BlockQueueEvent::AcceptedBufferedBlock(_, remaining_in_buffer) => {
                    if !self.is_established() {
                        info!(
                            "Catching up to tip of the chain (now at #{}, {} blocks remaining)",
                            self.blockchain.block_number(),
                            remaining_in_buffer
                        );
                    }
                }
                BlockQueueEvent::ReceivedMissingBlocks(_, _) => {
                    if !self.is_established() {
                        // When syncing a stopped chain, we want to immediately start a new head request
                        // after receiving blocks for the current epoch.
                        self.head_requests_time = None;
                    }
                }
            }
        }

        // Check consensus established state on changes.
        if let Some(event) = self.set_established(None) {
            self.events.send(event).ok(); // Ignore result.
        }

        // 2. Poll and push transactions (we check that consensus is established in the future itself).
        if self.tx_future.poll_unpin(cx).is_ready() {
            panic!("This future is driving an infinite Stream so it should never complete")
        };

        // 3. Poll any head requests if active.
        if let Some(ref mut head_requests) = self.head_requests {
            if let Poll::Ready(mut result) = head_requests.poll_unpin(cx) {
                // Reset head requests.
                self.head_requests = None;

                // Push unknown blocks to the block queue, trying to sync.
                for (block, peer) in result.unknown_blocks.drain(..) {
                    self.block_queue.push_block(block, peer);
                }

                // Update established state using the result.
                if let Some(event) = self.set_established(Some(result)) {
                    self.events.send(event).ok(); // Ignore result.
                }
            }
        }

        // 4. Update timer and poll it so the task gets woken when the timer runs out (at the latest)
        // The timer itself running out (producing an Instant) is of no interest to the execution. This poll method
        // was potentially awoken by the delays waker, but even then all there is to do is set up a new timer such
        // that it will wake this task again after another time frame has ellapsed. No interval was used as that
        // would periodically wake the task even though it might have just executed
        let _ = self.next_execution_timer.take();
        let mut timer = Box::pin(tokio::time::sleep(Self::CONSENSUS_POLL_TIMER));
        let _ = timer.poll_unpin(cx);
        self.next_execution_timer = Some(timer);

        // 5. Advance consensus and catch-up through head requests.
        self.request_heads();

        Poll::Pending
    }
}
