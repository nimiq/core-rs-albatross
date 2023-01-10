use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures::{FutureExt, StreamExt};
use nimiq_blockchain_proxy::BlockchainProxy;
use tokio::sync::broadcast::{channel as broadcast, Sender as BroadcastSender};
use tokio::time::Sleep;
use tokio_stream::wrappers::BroadcastStream;

use nimiq_blockchain::AbstractBlockchain;
use nimiq_database::Environment;
use nimiq_network_interface::{network::Network, request::request_handler};
use nimiq_zkp_component::zkp_component::ZKPComponentProxy;

use crate::consensus::head_requests::{HeadRequests, HeadRequestsResult};
use crate::messages::{
    RequestBatchSet, RequestBlock, RequestHead, RequestHistoryChunk, RequestMacroChain,
    RequestMissingBlocks,
};
use crate::sync::{syncer::LiveSyncPushEvent, syncer_proxy::SyncerProxy};

use self::consensus_proxy::ConsensusProxy;

pub mod consensus_proxy;
mod head_requests;

#[derive(Clone)]
pub enum ConsensusEvent {
    Established,
    Lost,
}

pub struct Consensus<N: Network> {
    pub blockchain: BlockchainProxy,
    pub network: Arc<N>,
    pub env: Environment,

    sync: SyncerProxy<N>,

    /// A Delay which exists purely for the waker on its poll to reactivate the task running Consensus::poll
    /// FIXME Remove this
    next_execution_timer: Option<Pin<Box<Sleep>>>,

    events: BroadcastSender<ConsensusEvent>,
    established_flag: Arc<AtomicBool>,
    head_requests: Option<HeadRequests<N>>,
    head_requests_time: Option<Instant>,

    min_peers: usize,

    zkp_proxy: ZKPComponentProxy<N>,
}

impl<N: Network> Consensus<N> {
    /// Minimum number of peers for consensus to be established.
    const MIN_PEERS_ESTABLISHED: usize = 3;

    /// Minimum number of block announcements extending the chain for consensus to be established.
    const MIN_BLOCKS_ESTABLISHED: usize = 5;

    /// Timeout after which head requests will be performed (again) to determine consensus
    /// established state and to advance the chain.
    const HEAD_REQUESTS_TIMEOUT: Duration = Duration::from_secs(5);

    /// Timeout after which the consensus is polled after it ran last
    ///
    /// TODO: Set appropriate duration
    /// FIXME Remove this
    const CONSENSUS_POLL_TIMER: Duration = Duration::from_secs(1);

    pub fn from_network(
        env: Environment,
        blockchain: BlockchainProxy,
        network: Arc<N>,
        syncer: SyncerProxy<N>,
        zkp_proxy: ZKPComponentProxy<N>,
    ) -> Self {
        Self::new(
            env,
            blockchain,
            network,
            syncer,
            Self::MIN_PEERS_ESTABLISHED,
            zkp_proxy,
        )
    }

    pub fn new(
        env: Environment,
        blockchain: BlockchainProxy,
        network: Arc<N>,
        syncer: SyncerProxy<N>,
        min_peers: usize,
        zkp_proxy: ZKPComponentProxy<N>,
    ) -> Self {
        let (tx, _rx) = broadcast(256);

        // IPTODO
        Self::init_network_request_receivers(&network, &blockchain);

        let established_flag = Arc::new(AtomicBool::new(false));

        let timer = Box::pin(tokio::time::sleep(Self::CONSENSUS_POLL_TIMER));

        Consensus {
            blockchain,
            network,
            env,
            sync: syncer,
            events: tx,
            next_execution_timer: Some(timer),
            established_flag,
            head_requests: None,
            head_requests_time: None,
            min_peers,
            zkp_proxy,
        }
    }

    fn init_network_request_receivers(network: &Arc<N>, blockchain: &BlockchainProxy) {
        let stream = network.receive_requests::<RequestMacroChain>();
        tokio::spawn(request_handler(network, stream, blockchain));

        let stream = network.receive_requests::<RequestBlock>();
        tokio::spawn(request_handler(network, stream, blockchain));

        let stream = network.receive_requests::<RequestMissingBlocks>();
        tokio::spawn(request_handler(network, stream, blockchain));

        let stream = network.receive_requests::<RequestHead>();
        tokio::spawn(request_handler(network, stream, blockchain));
        match blockchain {
            #[cfg(not(target_family = "wasm"))]
            BlockchainProxy::Full(blockchain) => {
                let stream = network.receive_requests::<RequestBatchSet>();
                tokio::spawn(request_handler(network, stream, blockchain));

                let stream = network.receive_requests::<RequestHistoryChunk>();
                tokio::spawn(request_handler(network, stream, blockchain));
            }
            BlockchainProxy::Light(_) => {}
        }
    }

    pub fn subscribe_events(&self) -> BroadcastStream<ConsensusEvent> {
        BroadcastStream::new(self.events.subscribe())
    }

    pub fn is_established(&self) -> bool {
        self.established_flag.load(Ordering::Acquire)
    }

    pub fn num_agents(&self) -> usize {
        self.sync.num_peers()
    }

    pub fn proxy(&self) -> ConsensusProxy<N> {
        ConsensusProxy {
            blockchain: self.blockchain.clone(),
            network: Arc::clone(&self.network),
            established_flag: Arc::clone(&self.established_flag),
            events: self.events.clone(),
        }
    }

    /// Forcefully sets consensus established, should be used for tests only.
    pub fn force_established(&mut self) {
        trace!("Consensus forcefully established.");
        self.established_flag.swap(true, Ordering::Release);

        // Also stop any other checks.
        self.head_requests = None;
        self.head_requests_time = None;
        // We don't care if anyone is listening.
        let _ = self.events.send(ConsensusEvent::Established);
    }

    /// Calculates and sets established state, returns a ConsensusEvent if the state changed.
    /// Once consensus is established, we can only lose it if we lose all our peers.
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
    fn check_established(
        &mut self,
        finished_head_request: Option<HeadRequestsResult<N>>,
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
                if self.sync.accepted_block_announcements() >= Self::MIN_BLOCKS_ESTABLISHED {
                    info!("Consensus established, number of accepted announcements satisfied.");
                    self.established_flag.swap(true, Ordering::Release);

                    // Also stop any other checks.
                    self.head_requests = None;
                    self.head_requests_time = None;
                    self.zkp_proxy
                        .request_zkp_from_peers(self.sync.peers(), false);
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
                            self.zkp_proxy
                                .request_zkp_from_peers(self.sync.peers(), false);
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
        // If there's no ongoing head request and we have at least one peer, check whether we should
        // start a new one.
        if self.head_requests.is_none() && (self.num_agents() > 0 || self.min_peers == 0) {
            // This is the case if `head_requests_time` is unset or the timeout is hit.
            let should_start_request = self
                .head_requests_time
                .map(|time| time.elapsed() >= Self::HEAD_REQUESTS_TIMEOUT)
                .unwrap_or(true);
            if should_start_request {
                debug!(
                    "Initiating head requests (to {} peers)",
                    self.sync.num_peers()
                );
                self.head_requests = Some(HeadRequests::new(
                    self.sync.peers(),
                    Arc::clone(&self.network),
                    self.blockchain.clone(),
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
        while let Poll::Ready(Some(event)) = self.sync.poll_next_unpin(cx) {
            match event {
                LiveSyncPushEvent::AcceptedAnnouncedBlock(_) => {
                    // Reset the head request timer when an announced block was accepted.
                    self.head_requests_time = Some(Instant::now());
                }
                LiveSyncPushEvent::AcceptedBufferedBlock(_, remaining_in_buffer) => {
                    if !self.is_established() {
                        // Note: this output is parsed by our testing infrastructure (specifically devnet.sh),
                        // so please test that nothing breaks in there if you change this.
                        let block_number = {
                            let blockchain = self.blockchain.read();
                            blockchain.block_number()
                        };

                        info!(
                            "Catching up to tip of the chain (now at #{}, {} blocks remaining)",
                            block_number, remaining_in_buffer
                        );

                        if remaining_in_buffer == 0 {
                            self.head_requests_time = None;
                        }
                    }
                }
                LiveSyncPushEvent::ReceivedMissingBlocks(_, _) => {
                    if !self.is_established() {
                        // When syncing a stopped chain, we want to immediately start a new head request
                        // after receiving blocks for the current epoch.
                        self.head_requests_time = None;
                    }
                }
                LiveSyncPushEvent::RejectedBlock(hash) => {
                    warn!("Rejected block {}", hash);
                }
            }
        }

        // Check consensus established state on changes.
        if let Some(event) = self.check_established(None) {
            if let Err(error) = self.events.send(event) {
                error!(%error, "Could not send established event")
            }
        }

        // 2. Poll any head requests if active.
        if let Some(ref mut head_requests) = self.head_requests {
            if let Poll::Ready(mut result) = head_requests.poll_unpin(cx) {
                // Reset head requests.
                self.head_requests = None;

                // Push unknown blocks to the block queue, trying to sync.
                for (block, peer_id) in result.unknown_blocks.drain(..) {
                    self.sync.push_block(block, peer_id, None);
                }

                // Update established state using the result.
                if let Some(event) = self.check_established(Some(result)) {
                    if let Err(error) = self.events.send(event) {
                        error!(%error, "Could not send established event after handling head requests");
                    }
                }
            }
        }

        // 3. Update timer and poll it so the task gets woken when the timer runs out (at the latest)
        // The timer itself running out (producing an Instant) is of no interest to the execution. This poll method
        // was potentially awoken by the delays waker, but even then all there is to do is set up a new timer such
        // that it will wake this task again after another time frame has elapsed. No interval was used as that
        // would periodically wake the task even though it might have just executed
        let mut timer = Box::pin(tokio::time::sleep(Self::CONSENSUS_POLL_TIMER));
        // If the sleep wasn't pending anymore, it didn't register us with the waker, but we need that.
        assert!(timer.poll_unpin(cx) == Poll::Pending);
        self.next_execution_timer = Some(timer);

        // 4. Advance consensus and catch-up through head requests.
        self.request_heads();

        Poll::Pending
    }
}
