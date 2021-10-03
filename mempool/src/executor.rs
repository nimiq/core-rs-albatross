use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use std::sync::Arc;

use futures::task::{Context, Poll};
use futures::{stream::BoxStream, Future, StreamExt};
use parking_lot::RwLock;

use nimiq_blockchain::Blockchain;
use nimiq_network_interface::network::{Network, Topic};
use nimiq_network_interface::prelude::MsgAcceptance;
use nimiq_transaction::Transaction;

use crate::filter::MempoolFilter;
use crate::mempool::MempoolState;
use crate::verify::{verify_tx, ReturnCode};

const CONCURRENT_VERIF_TASKS: u32 = 1000;

/// Transaction topic for the MempoolExecutor to request transactions from the network
#[derive(Clone, Debug, Default)]
pub struct TransactionTopic;

impl Topic for TransactionTopic {
    type Item = Transaction;

    const BUFFER_SIZE: usize = 1024;
    const NAME: &'static str = "transactions";
    const VALIDATE: bool = true;
}

pub(crate) struct MempoolExecutor<N: Network> {
    // Blockchain reference
    blockchain: Arc<RwLock<Blockchain>>,

    // The mempool state: the data structure where the transactions are stored
    state: Arc<RwLock<MempoolState>>,

    // Mempool filter
    filter: Arc<RwLock<MempoolFilter>>,

    // Ongoing verification tasks counter
    verification_tasks: Arc<AtomicU32>,

    // Reference to the network, to alow for message validation
    network: Arc<N>,

    // Transaction stream that is used to listen to transactions from the network
    txn_stream: BoxStream<'static, (Transaction, <N as Network>::PubsubId)>,
}

impl<N: Network> MempoolExecutor<N> {
    pub async fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        state: Arc<RwLock<MempoolState>>,
        filter: Arc<RwLock<MempoolFilter>>,
        network: Arc<N>,
    ) -> Self {
        let txn_stream = network.subscribe::<TransactionTopic>().await.unwrap();
        Self {
            blockchain,
            state,
            filter,
            network,
            verification_tasks: Arc::new(AtomicU32::new(0)),
            txn_stream,
        }
    }

    pub fn with_txn_stream(
        blockchain: Arc<RwLock<Blockchain>>,
        state: Arc<RwLock<MempoolState>>,
        filter: Arc<RwLock<MempoolFilter>>,
        network: Arc<N>,
        txn_stream: BoxStream<'static, (Transaction, <N as Network>::PubsubId)>,
    ) -> Self {
        Self {
            blockchain,
            state,
            filter,
            network,
            verification_tasks: Arc::new(AtomicU32::new(0)),
            txn_stream,
        }
    }
}

impl<N: Network> Future for MempoolExecutor<N> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while let Poll::Ready(Some((tx, pubsub_id))) = self.txn_stream.as_mut().poll_next_unpin(cx)
        {
            if self.verification_tasks.fetch_add(0, AtomicOrdering::SeqCst)
                == CONCURRENT_VERIF_TASKS
            {
                log::debug!("Reached the max number of verification tasks");
                continue;
            }

            let blockchain = Arc::clone(&self.blockchain);
            let mempool_state = Arc::clone(&self.state);
            let filter = Arc::clone(&self.filter);
            let tasks_count = Arc::clone(&self.verification_tasks);

            let network = Arc::clone(&self.network);

            // Spawn the transaction verification task
            tokio::task::spawn(async move {
                tasks_count.fetch_add(1, AtomicOrdering::SeqCst);

                let rc = verify_tx(&tx, blockchain, Arc::clone(&mempool_state), filter);

                let acceptance = if rc == ReturnCode::Accepted {
                    mempool_state.write().put(&tx);
                    MsgAcceptance::Accept
                } else {
                    MsgAcceptance::Ignore
                };

                if let Err(e) = network.validate_message(pubsub_id, acceptance).await {
                    log::trace!("failed to validate_message for tx: {:?}", e);
                };

                tasks_count.fetch_sub(1, AtomicOrdering::SeqCst);
            });
        }

        Poll::Pending
    }
}
