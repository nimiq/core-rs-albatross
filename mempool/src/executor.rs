use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use std::sync::Arc;

use futures::task::{Context, Poll};
use futures::{stream::BoxStream, Future, StreamExt};

use parking_lot::RwLock;

use nimiq_account::{Account, BasicAccount};

use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_hash::Hash;

use nimiq_network_interface::network::{Network, Topic};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::Transaction;

use crate::filter::MempoolFilter;
use crate::mempool::MempoolState;

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

/// Return code for the Mempool executor future
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReturnCode {
    /// Fee is too low
    FeeTooLow,
    /// Transaction is invalid
    Invalid,
    /// Transaction is accepted
    Accepted,
    /// Transaction is already known
    Known,
    /// Transaction is filtered
    Filtered,
    /// Transaction signature is correct
    SignOk,
}

pub(crate) struct MempoolExecutor<N: Network> {
    // Ongoing verification tasks counter
    verification_tasks: Arc<AtomicU32>,

    // Blockchain reference
    blockchain: Arc<RwLock<Blockchain>>,

    // The mempool state: the data structure where the transactions are stored
    state: Arc<RwLock<MempoolState>>,

    // Network id
    network_id: NetworkId,

    // Transaction stream that is used to listen to transactions from the network
    txn_stream: BoxStream<'static, (Transaction, <N as Network>::PubsubId)>,

    // Mempool filter
    filter: Arc<RwLock<MempoolFilter>>,
}

impl<N: Network> MempoolExecutor<N> {
    pub async fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        state: Arc<RwLock<MempoolState>>,
        filter: Arc<RwLock<MempoolFilter>>,
        network: Arc<N>,
        network_id: NetworkId,
    ) -> Self {
        let txn_stream = network.subscribe::<TransactionTopic>().await.unwrap();
        Self {
            blockchain,
            state,
            filter,
            verification_tasks: Arc::new(AtomicU32::new(0)),
            network_id,
            txn_stream,
        }
    }

    pub fn with_txn_stream(
        blockchain: Arc<RwLock<Blockchain>>,
        state: Arc<RwLock<MempoolState>>,
        filter: Arc<RwLock<MempoolFilter>>,
        network_id: NetworkId,
        txn_stream: BoxStream<'static, (Transaction, <N as Network>::PubsubId)>,
    ) -> Self {
        Self {
            blockchain,
            state,
            verification_tasks: Arc::new(AtomicU32::new(0)),
            filter,
            network_id,
            txn_stream,
        }
    }
}

impl<N: Network> Future for MempoolExecutor<N> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while let Poll::Ready(Some((tx, _pubsub_id))) = self.txn_stream.as_mut().poll_next_unpin(cx)
        {
            log::debug!("Received new transaction");

            if self.verification_tasks.fetch_add(0, AtomicOrdering::SeqCst)
                == CONCURRENT_VERIF_TASKS
            {
                log::debug!("Reached the max number of verification tasks");
                return Poll::Pending;
            }

            // Obtain the network id from the blockchain
            let network_id = self.network_id;
            let blockchain = Arc::clone(&self.blockchain);
            let tasks_count = Arc::clone(&self.verification_tasks);
            let mempool_state = Arc::clone(&self.state);
            let filter = Arc::clone(&self.filter);

            // Check if we already know the transaction
            {
                if mempool_state.read().contains(&tx.hash()) {
                    //We already know this transaction, no need to process
                    log::debug!("Transaction is already known ");
                    return Poll::Pending;
                }
            }

            // Check if the transaction is going to be filtered.
            // Do it in a separate scope to drop the lock.
            {
                let filter = self.filter.read();
                if !filter.accepts_transaction(&tx) || filter.blacklisted(&tx.hash()) {
                    log::debug!("Transaction filtered");
                    return Poll::Pending;
                }
            }

            log::debug!("Spawning a new verification task");

            // Spawn the transaction verification task
            tokio::task::spawn(async move {
                log::debug!("Starting execution of new verif task");

                tasks_count.fetch_add(1, AtomicOrdering::SeqCst);

                let mut transaction = tx.clone();

                // 1. Verify signature in a new blocking verification task, join handle after step 4 below
                let sign_verification_handle = tokio::task::spawn_blocking(move || {
                    if let Err(err) = transaction.verify_mut(network_id) {
                        log::debug!("Intrinsic tx verification Failed {:?}", err);
                        return ReturnCode::Invalid;
                    }
                    ReturnCode::SignOk
                });

                // Check the result of the sign verification for the tx
                match sign_verification_handle.await {
                    Ok(rc) => {
                        if rc == ReturnCode::Invalid {
                            // If signature verification failed we just return
                            return ReturnCode::Invalid;
                        }
                    }
                    Err(_err) => {
                        return ReturnCode::Invalid;
                    }
                };

                // 2. Acquire Blockchain read lock
                let blockchain = blockchain.read();

                // 3. Check Validity Window and already included
                let block_height = blockchain.block_number() + 1;

                if !tx.is_valid_at(block_height) {
                    log::debug!("Transaction invalid at block {}", block_height);
                    tasks_count.fetch_sub(1, AtomicOrdering::SeqCst);
                    return ReturnCode::Invalid;
                }

                if blockchain.contains_tx_in_validity_window(&tx.hash()) {
                    log::debug!("Transaction has already been mined");
                    tasks_count.fetch_sub(1, AtomicOrdering::SeqCst);
                    return ReturnCode::Invalid;
                }

                // 4. Sequentialize per Sender to Check Balances and acquire the upgradable from the blockchain),
                // Perform all balances checks
                let sender_account = match blockchain.get_account(&tx.sender) {
                    None => {
                        log::debug!(
                            "There is no account for this sender in the blockchain {}",
                            tx.sender.to_user_friendly_address()
                        );
                        return ReturnCode::Invalid;
                    }
                    Some(account) => account,
                };

                // Get recipient account to later check against filter rules.
                let recipient_account = match blockchain.get_account(&tx.recipient) {
                    None => Account::Basic(BasicAccount {
                        balance: Coin::ZERO,
                    }),
                    Some(x) => x,
                };

                let blockchain_sender_balance = sender_account.balance();
                let blockchain_recipient_balance = recipient_account.balance();

                // Read the pending transactions balance
                let mut sender_current_balance = Coin::ZERO;
                let mut recipient_current_balance = blockchain_recipient_balance;

                let mut mempool_state = mempool_state.write();

                if let Some(sender_state) = mempool_state.state_by_sender.get_mut(&tx.sender) {
                    sender_current_balance = sender_state.total;
                }

                if let Some(recipient_state) = mempool_state.state_by_sender.get_mut(&tx.recipient)
                {
                    // We found the recipient in the mempool. Subtract the mempool balance from the recipient balance
                    recipient_current_balance -= recipient_state.total;
                }

                // Calculate the new balance assuming we add this transaction to the mempool
                let sender_in_fly_balance = tx.total_value().unwrap() + sender_current_balance;
                let recipient_in_fly_balance =
                    tx.total_value().unwrap() + recipient_current_balance;

                // Check the balance against filters
                // Do it in a separate scope to drop the lock.
                {
                    let filter = filter.read();

                    if !filter.accepts_sender_balance(
                        &tx,
                        blockchain_sender_balance,
                        sender_in_fly_balance,
                    ) {
                        log::debug!(
                            "Transaction filtered: Not accepting transaction due to sender balance"
                        );
                        return ReturnCode::Filtered;
                    }

                    if !filter.accepts_recipient_balance(
                        &tx,
                        blockchain_recipient_balance,
                        recipient_in_fly_balance,
                    ) {
                        log::debug!("Transaction filtered: Not accepting transaction due to recipient balance");
                        return ReturnCode::Filtered;
                    }
                }

                if sender_in_fly_balance <= blockchain_sender_balance {
                    // 6. Add transaction to the mempool
                    log::debug!(" Accepting new transaction");

                    mempool_state.put(&tx);
                } else {
                    log::debug!(
                        "Dropped because sum of txs in mempool is larger than the account balance"
                    );
                }

                // 7. Drop locks (mempool then Blockchain)
                drop(mempool_state);
                drop(blockchain);
                tasks_count.fetch_sub(1, AtomicOrdering::SeqCst);
                ReturnCode::Accepted
            });
        }

        Poll::Pending
    }
}
