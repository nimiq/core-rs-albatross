use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::{ready, stream::BoxStream, StreamExt};
use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use nimiq_blockchain::Blockchain;
use nimiq_network_interface::network::{MsgAcceptance, Network, Topic};
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::Transaction;

use crate::filter::MempoolFilter;
use crate::mempool_state::MempoolState;
use crate::mempool_transactions::TxPriority;
use crate::verify::{verify_tx, VerifyErr};

const CONCURRENT_VERIF_TASKS: u32 = 1000;

pub(crate) struct MempoolExecutor<N: Network, T: Topic + Unpin + Sync> {
    // Blockchain reference
    blockchain: Arc<RwLock<Blockchain>>,

    // The mempool state: the data structure where the transactions are stored
    state: Arc<RwLock<MempoolState>>,

    // Mempool filter
    filter: Arc<RwLock<MempoolFilter>>,

    // Ongoing verification tasks counter
    verification_tasks: Arc<AtomicU32>,

    // Reference to the network, to allow for message validation
    network: Arc<N>,

    // Network ID, used for tx verification
    network_id: Arc<NetworkId>,

    // Transaction stream that is used to listen to transactions from the network
    txn_stream: BoxStream<'static, (Transaction, <N as Network>::PubsubId)>,

    // Phantom data for the unused type T
    _phantom: PhantomData<T>,
}

impl<N: Network, T: Topic + Unpin + Sync> MempoolExecutor<N, T> {
    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        state: Arc<RwLock<MempoolState>>,
        filter: Arc<RwLock<MempoolFilter>>,
        network: Arc<N>,
        txn_stream: BoxStream<'static, (Transaction, <N as Network>::PubsubId)>,
        verification_tasks: Arc<AtomicU32>,
    ) -> Self {
        Self {
            blockchain: blockchain.clone(),
            state,
            filter,
            network,
            network_id: Arc::new(blockchain.read().network_id),
            verification_tasks,
            txn_stream,
            _phantom: PhantomData,
        }
    }
}

impl<N: Network, T: Topic + Unpin + Sync> Future for MempoolExecutor<N, T> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while let Some((tx, pubsub_id)) = ready!(self.txn_stream.as_mut().poll_next_unpin(cx)) {
            if self.verification_tasks.fetch_add(0, AtomicOrdering::SeqCst)
                >= CONCURRENT_VERIF_TASKS
            {
                log::debug!("Reached the max number of verification tasks");
                continue;
            }

            let blockchain = Arc::clone(&self.blockchain);
            let mempool_state = Arc::clone(&self.state);
            let filter = Arc::clone(&self.filter);
            let tasks_count = Arc::clone(&self.verification_tasks);
            let network_id = Arc::clone(&self.network_id);
            let network = Arc::clone(&self.network);

            // Spawn the transaction verification task
            tokio::task::spawn(async move {
                tasks_count.fetch_add(1, AtomicOrdering::SeqCst);

                // Verifying and pushing the TX in a separate scope to drop the lock that is returned by
                // the verify_tx function immediately
                let acceptance = {
                    let verify_tx_ret =
                        verify_tx(&tx, blockchain, network_id, &mempool_state, filter).await;

                    match verify_tx_ret {
                        Ok(mempool_state_lock) => {
                            RwLockUpgradableReadGuard::upgrade(mempool_state_lock)
                                .put(&tx, TxPriority::MediumPriority);
                            MsgAcceptance::Accept
                        }
                        // Reject the message if signature verification fails or transaction is invalid
                        // for current validation window
                        Err(VerifyErr::InvalidSignature) => MsgAcceptance::Reject,
                        Err(VerifyErr::InvalidTxWindow) => MsgAcceptance::Reject,
                        Err(_) => MsgAcceptance::Ignore,
                    }
                };

                network.validate_message::<T>(pubsub_id, acceptance);

                tasks_count.fetch_sub(1, AtomicOrdering::SeqCst);
            });
        }

        // We have exited the loop, so poll_next() must have returned Poll::Ready(None).
        // Thus, we terminate the executor future.
        Poll::Ready(())
    }
}
