use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::{
        atomic::{AtomicU32, Ordering as AtomicOrdering},
        Arc,
    },
    task::{Context, Poll},
};

use futures::{ready, stream::BoxStream, StreamExt};
use nimiq_blockchain::Blockchain;
use nimiq_network_interface::network::{MsgAcceptance, Network, Topic};
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::Transaction;
use nimiq_utils::spawn;
use parking_lot::RwLock;

use crate::{
    filter::MempoolFilter,
    mempool_state::MempoolState,
    mempool_transactions::TxPriority,
    verify::{verify_tx, VerifyErr},
};

const CONCURRENT_VERIF_TASKS: u32 = 10000;

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
    network_id: NetworkId,

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
            blockchain: Arc::clone(&blockchain),
            state,
            filter,
            network,
            network_id: blockchain.read().network_id,
            verification_tasks,
            txn_stream,
            _phantom: PhantomData,
        }
    }
}

impl<N: Network, T: Topic + Unpin + Sync> Future for MempoolExecutor<N, T> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        struct Decrementer(Arc<AtomicU32>);
        impl Drop for Decrementer {
            fn drop(&mut self) {
                self.0.fetch_sub(1, AtomicOrdering::Relaxed);
            }
        }

        while let Some((tx, pubsub_id)) = ready!(self.txn_stream.as_mut().poll_next_unpin(cx)) {
            let decrement = Decrementer(Arc::clone(&self.verification_tasks));
            if self
                .verification_tasks
                .fetch_add(1, AtomicOrdering::Relaxed)
                >= CONCURRENT_VERIF_TASKS
            {
                log::debug!("Reached the max number of verification tasks");
                continue;
            }

            let blockchain = Arc::clone(&self.blockchain);
            let mempool_state = Arc::clone(&self.state);
            let filter = Arc::clone(&self.filter);
            let network = Arc::clone(&self.network);
            let network_id = self.network_id;

            // Spawn the transaction verification task
            spawn(async move {
                let verify_tx_ret = verify_tx(
                    &tx,
                    blockchain,
                    network_id,
                    &mempool_state,
                    filter,
                    TxPriority::Medium,
                )
                .await;

                let acceptance = match verify_tx_ret {
                    Ok(_) => MsgAcceptance::Accept,
                    // Reject the message if signature verification fails or transaction is invalid
                    // for current validation window
                    Err(VerifyErr::InvalidTransaction(_)) => MsgAcceptance::Reject,
                    Err(VerifyErr::AlreadyIncluded) => MsgAcceptance::Reject,
                    Err(_) => MsgAcceptance::Ignore,
                };

                network.validate_message::<T>(pubsub_id, acceptance);

                drop(decrement);
            });
        }

        // We have exited the loop, so poll_next() must have returned Poll::Ready(None).
        // Thus, we terminate the executor future.
        Poll::Ready(())
    }
}
