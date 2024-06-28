use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{stream::BoxStream, Future, Stream, StreamExt};
use log::{debug, trace, warn};
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_consensus::{Consensus, ConsensusEvent, ConsensusProxy};
use nimiq_mempool::{config::MempoolConfig, mempool::Mempool};
use nimiq_network_interface::network::Network;
use nimiq_utils::spawn::spawn;
use parking_lot::RwLock;
#[cfg(feature = "metrics")]
use tokio_metrics::TaskMonitor;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};

/// Emits a mempool event after a blockchain event has been processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolEvent(BlockchainEvent);

impl From<MempoolEvent> for BlockchainEvent {
    fn from(event: MempoolEvent) -> Self {
        event.0
    }
}

/// This struct wraps the mempool and is responsible for updating the mempool based on external events.
/// It can be spawned as a task individually or be polled by the validator as a stream.
pub struct MempoolTask<N: Network> {
    pub consensus: ConsensusProxy<N>,
    pub blockchain: Arc<RwLock<Blockchain>>,

    consensus_event_rx: BroadcastStream<ConsensusEvent>,
    blockchain_event_rx: BoxStream<'static, BlockchainEvent>,

    pub mempool: Arc<Mempool>,
    mempool_active: bool,
    #[cfg(feature = "metrics")]
    mempool_monitor: TaskMonitor,
    #[cfg(feature = "metrics")]
    control_mempool_monitor: TaskMonitor,
}

impl<N: Network> MempoolTask<N> {
    pub fn new(
        consensus: &Consensus<N>,
        blockchain: Arc<RwLock<Blockchain>>,
        mempool_config: MempoolConfig,
    ) -> Self {
        let consensus_event_rx = consensus.subscribe_events();

        let blockchain_rg = blockchain.read();
        let blockchain_event_rx = blockchain_rg.notifier_as_stream();
        drop(blockchain_rg);

        let mempool = Arc::new(Mempool::new(Arc::clone(&blockchain), mempool_config));
        let mempool_active = false;

        Self {
            consensus: consensus.proxy(),
            blockchain,

            consensus_event_rx,
            blockchain_event_rx,

            mempool: Arc::clone(&mempool),
            mempool_active,
            #[cfg(feature = "metrics")]
            mempool_monitor: TaskMonitor::new(),
            #[cfg(feature = "metrics")]
            control_mempool_monitor: TaskMonitor::new(),
        }
    }

    pub fn mempool(&self) -> Arc<Mempool> {
        Arc::clone(&self.mempool)
    }

    #[cfg(feature = "metrics")]
    pub fn get_mempool_monitor(&self) -> TaskMonitor {
        self.mempool_monitor.clone()
    }

    #[cfg(feature = "metrics")]
    pub fn get_control_mempool_monitor(&self) -> TaskMonitor {
        self.control_mempool_monitor.clone()
    }

    fn init_mempool(&mut self) {
        if self.mempool_active || !self.consensus.is_ready_for_validation() {
            return;
        }

        let mempool = Arc::clone(&self.mempool);
        let network = Arc::clone(&self.consensus.network);
        #[cfg(not(feature = "metrics"))]
        spawn({
            async move {
                // The mempool is not updated while consensus is lost.
                // Thus, we need to check all transactions if they are still valid.
                mempool.cleanup();
                mempool.start_executors(network, None, None).await;
            }
        });
        #[cfg(feature = "metrics")]
        spawn({
            let mempool_monitor = self.mempool_monitor.clone();
            let ctrl_mempool_monitor = self.control_mempool_monitor.clone();
            async move {
                // The mempool is not updated while consensus is lost.
                // Thus, we need to check all transactions if they are still valid.
                mempool.cleanup();

                mempool
                    .start_executors(network, Some(mempool_monitor), Some(ctrl_mempool_monitor))
                    .await;
            }
        });

        self.mempool_active = true;
    }

    fn pause(&mut self) {
        if !self.mempool_active {
            return;
        }

        let mempool = Arc::clone(&self.mempool);
        let network = Arc::clone(&self.consensus.network);
        spawn(async move {
            mempool.stop_executors(network).await;
        });

        self.mempool_active = false;
    }

    fn on_blockchain_event(&mut self, event: &BlockchainEvent) {
        match event {
            BlockchainEvent::HistoryAdopted(_) => {
                // Mempool updates are only done once we are synced.
                if self.consensus.is_ready_for_validation() {
                    self.mempool.cleanup();
                    debug!("Performed a mempool clean up because new history was adopted");
                }
            }
            BlockchainEvent::Extended(hash) => {
                // Mempool updates are only done once we are synced.
                if self.consensus.is_ready_for_validation() {
                    let block = self
                        .consensus
                        .blockchain
                        .read()
                        .get_block(hash, true)
                        .expect("Head block not found");

                    self.mempool
                        .update(&vec![(hash.clone(), block)], [].as_ref());
                }
            }
            BlockchainEvent::Rebranched(old_chain, new_chain) => {
                // Mempool updates are only done once we are synced.
                if self.consensus.is_ready_for_validation() {
                    self.mempool.update(new_chain, old_chain);
                }
            }
            _ => {
                // Nothing to do here.
            }
        }
    }
}

impl<N: Network> Stream for MempoolTask<N> {
    type Item = MempoolEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Process consensus updates.
        // Start mempool as soon as we have consensus and can enforce the validity window.
        // Stop the mempool if we lose consensus or cannot enforce the validity window.
        while let Poll::Ready(Some(event)) = self.consensus_event_rx.poll_next_unpin(cx) {
            match event {
                Ok(ConsensusEvent::Established {
                    synced_validity_window: true,
                }) => self.init_mempool(),
                Ok(ConsensusEvent::Lost)
                | Ok(ConsensusEvent::Established {
                    synced_validity_window: false,
                }) => self.pause(),
                Err(BroadcastStreamRecvError::Lagged(num)) => {
                    warn!("Consensus event stream lagging behind by {} messages", num);
                }
            }
        }

        // Process blockchain updates.
        if let Poll::Ready(Some(event)) = self.blockchain_event_rx.poll_next_unpin(cx) {
            trace!(?event, is_active = self.mempool_active, "blockchain event");
            self.on_blockchain_event(&event);
            return Poll::Ready(Some(MempoolEvent(event)));
        }

        Poll::Pending
    }
}

impl<N: Network> Future for MempoolTask<N> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Poll until the stream is exhausted.
        while let Poll::Ready(Some(_event)) = self.poll_next_unpin(cx) {}

        Poll::Pending
    }
}
