use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Error;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use futures::future::{AbortHandle, Abortable};
use futures::task::{Context, Poll};
use futures::{stream::BoxStream, Future, StreamExt};
use keyed_priority_queue::KeyedPriorityQueue;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use beserial::Serialize;
use nimiq_account::Account;
use nimiq_block::Block;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_network_interface::network::{Network, Topic};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::Transaction;

use crate::config::MempoolConfig;
use crate::filter::MempoolFilter;

const CONCURRENT_VERIF_TASKS: u32 = 1000;

#[derive(Clone, Debug, Default)]
pub struct TransactionTopic;

impl Topic for TransactionTopic {
    type Item = Transaction;

    const BUFFER_SIZE: usize = 1024;
    const NAME: &'static str = "transactions";
    const VALIDATE: bool = true;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReturnCode {
    FeeTooLow,
    Invalid,
    Accepted,
    Known,
    Filtered,
    SignOk,
}

pub struct Mempool {
    // Blockchain reference
    blockchain: Arc<RwLock<Blockchain>>,

    // The mempool state: the data structure where the transactions are stored
    state: Arc<RwLock<MemPoolState>>,

    // Mempool filter
    filter: Arc<RwLock<MempoolFilter>>,

    // Mempool executor handle used to stop the executor
    executor_handle: Mutex<Option<AbortHandle>>,
}

impl Mempool {
    pub fn new(blockchain: Arc<RwLock<Blockchain>>, config: MempoolConfig) -> Self {
        let state = MemPoolState {
            transactions_by_fee: KeyedPriorityQueue::new(),
            transactions_by_age: KeyedPriorityQueue::new(),
            state_by_sender: HashMap::new(),
        };

        let state = Arc::new(RwLock::new(state));

        Self {
            blockchain: Arc::clone(&blockchain),
            state: Arc::clone(&state),
            filter: Arc::new(RwLock::new(MempoolFilter::new(
                config.filter_rules,
                config.filter_limit,
            ))),
            executor_handle: Mutex::new(None),
        }
    }

    pub async fn start_executor<N: Network>(&self, network: Arc<N>) {
        if self.executor_handle.lock().unwrap().is_some() {
            //If we already have an executor running, dont do anything
            return;
        }

        let network_id = self.blockchain.read().network_id;
        let mempool_executor = MempoolExecutor::new(
            Arc::clone(&self.blockchain),
            Arc::clone(&self.state),
            Arc::clone(&self.filter),
            Arc::clone(&network),
            network_id,
        )
        .await;

        // Start the executor and obtain its handle
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let _ = Abortable::new(tokio::spawn(mempool_executor), abort_registration);

        //Set the executor handle
        *self.executor_handle.lock().unwrap() = Some(abort_handle);
    }

    pub fn is_filtered(&self, hash: &Blake2bHash) -> bool {
        self.filter.read().blacklisted(hash)
    }

    pub async fn start_executor_with_txn_stream<N: Network>(
        &self,
        txn_stream: BoxStream<'static, (Transaction, <N as Network>::PubsubId)>,
    ) {
        if self.executor_handle.lock().unwrap().is_some() {
            //If we already have an executor running, dont do anything
            return;
        }

        let network_id = self.blockchain.read().network_id;
        let mempool_executor = MempoolExecutor::<N>::with_txn_stream(
            Arc::clone(&self.blockchain),
            Arc::clone(&self.state),
            Arc::clone(&self.filter),
            network_id,
            txn_stream,
        );

        // Start the executor and obtain its handle
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let _ = Abortable::new(tokio::spawn(mempool_executor), abort_registration);

        //Set the executor handle
        *self.executor_handle.lock().unwrap() = Some(abort_handle);
    }

    pub fn stop_executor(&self) {
        let mut handle = self.executor_handle.lock().unwrap();

        if handle.is_none() {
            //If there isnt any executor running we return
            return;
        }

        // Stop the executor
        handle.take().expect("Expected an executor handle").abort();
    }

    // Return the highest fee per byte up to max_count transactions and removes them from the mempool
    pub fn get_transactions(&self, max_bytes: usize) -> Result<Vec<Transaction>, Error> {
        let mut tx_vec = vec![];

        let state = self.state.upgradable_read();

        if state.transactions_by_fee.is_empty() {
            log::debug!("Requesting txns and there are no txns in the mempool ");
            return Ok(tx_vec);
        }

        let mut size = 0_usize;

        let mut mempool_state_upgraded = RwLockUpgradableReadGuard::upgrade(state);

        loop {
            // Remove the transaction from the mempool
            let tx: Transaction;

            if let Some((transaction, _ord)) = mempool_state_upgraded.transactions_by_fee.peek() {
                tx = transaction.clone();
            } else {
                break;
            }

            size += tx.serialized_size();

            if size > max_bytes {
                break;
            }

            mempool_state_upgraded.transactions_by_fee.pop();

            // We should also remove it from the transactions by age
            mempool_state_upgraded
                .transactions_by_age
                .remove(&tx)
                .unwrap();

            // Update the per sender in fly state
            if let Some(pending_state) = mempool_state_upgraded.state_by_sender.get_mut(&tx.sender)
            {
                pending_state.total -= tx.total_value().unwrap();
                pending_state.txns.remove(&tx.hash()).unwrap();

                if pending_state.txns.is_empty() {
                    mempool_state_upgraded.state_by_sender.remove(&tx.sender);
                }
            }

            // Push the transaction to our output vector
            tx_vec.push(tx.clone());
        }

        log::debug!("Returning vector with {} transactions", tx_vec.len());

        drop(mempool_state_upgraded);

        Ok(tx_vec)
    }

    // During a Blockchain extend event a new block is mined which implies that
    //
    //      1. Existing transactions in the mempool can become invalidated because:
    //          A. They are no longer valid at the new block height (aging)
    //          B. Some were already mined
    //
    //      2. A transaction, that we didn't know about, from a known sender could be included in the blockchain, which implies:
    //          A. We need to update the sender balances in our mempool because some txns in or mempool could become invalid
    //
    //      1.B and 2.A can be iterated over the txs in the adopted blocks, that is, it is not
    //      necessary to iterate all transactions in the mempool
    //
    pub fn mempool_update(
        &self,
        adopted_blocks: &[(Blake2bHash, Block)],
        reverted_blocks: &[(Blake2bHash, Block)],
    ) {
        let mut txs_to_remove = Vec::new();

        // Acquire the mempool and blockchain locks
        let mut mempool_state = self.state.write();
        let blockchain = self.blockchain.read();

        let block_height = blockchain.block_number() + 1;

        // First remove the transactions that are no longer valid due to age
        while let Some((tx, _)) = mempool_state.transactions_by_age.peek() {
            if !tx.is_valid_at(block_height) {
                txs_to_remove.push(tx.clone());
                mempool_state.transactions_by_age.pop();
            } else {
                // No need to process more transactions, since we arrived to the oldest one that is valid
                break;
            }
        }

        // Update the in-fly sender balance
        for tx in txs_to_remove.iter() {
            if let Some(sender_state) = mempool_state.state_by_sender.get_mut(&tx.sender) {
                // Update the in fly per sender balance
                sender_state.total -= tx.total_value().unwrap();
                sender_state.txns.remove(&tx.hash());

                if sender_state.txns.is_empty() {
                    mempool_state.state_by_sender.remove(&tx.sender);
                }
            }
        }

        // Remove the aged transactions from the mempool
        while let Some(tx) = txs_to_remove.pop() {
            mempool_state.transactions_by_fee.remove(&tx);
        }

        // Now iterate over the transactions in the adopted blocks:
        //  if transaction was known:
        //      remove it from the mempool
        //  else
        //      update the sender balances
        //      (some transactions could become invalid )
        //
        for (_, block) in adopted_blocks {
            let transactions = block.transactions();

            if transactions.is_none() {
                continue;
            }

            for tx in transactions.unwrap() {
                if let Some(sender_state) = mempool_state.state_by_sender.get_mut(&tx.sender) {
                    // Check if we already know this transaction
                    if sender_state.txns.contains_key(&tx.hash()) {
                        // A known transaction was mined so we need to remove it from our balances
                        sender_state.total -= tx.total_value().unwrap();
                        sender_state.txns.remove(&tx.hash());

                        if sender_state.txns.is_empty() {
                            mempool_state.state_by_sender.remove(&tx.sender);
                        }

                        txs_to_remove.push(tx.clone());
                        continue;
                    } else {
                        // This an unknown transaction from a known sender, we need to update our senders balance
                        // and some transactions could become invalid

                        // Lets read the balance from the blockchain
                        let sender_account = blockchain.get_account(&tx.sender).unwrap();
                        let blockchain_balance = sender_account.balance();

                        if sender_state.total <= blockchain_balance {
                            // Sender still has enough funds to pay for all pending transactions
                            continue;
                        } else {
                            let mut new_total = Coin::ZERO;

                            // TODO: We could have per sender transactions ordered by fee to try to keep the ones with higher fee
                            let drained: HashMap<Blake2bHash, Transaction> = sender_state
                                .txns
                                .drain_filter(|_hash, old_tx| {
                                    if old_tx.total_value().unwrap() + new_total
                                        <= blockchain_balance
                                    {
                                        true
                                    } else {
                                        new_total -= old_tx.total_value().unwrap();
                                        txs_to_remove.push(tx.clone());
                                        false
                                    }
                                })
                                .collect();
                            sender_state.total = new_total;
                            sender_state.txns = drained;
                        }
                    }
                    // If the sender txns are empty remove them
                    if sender_state.txns.is_empty() {
                        mempool_state.state_by_sender.remove(&tx.sender);
                    }
                } else {
                    // This is a new transaction from a sender that we did not know about,
                    // so we don't care, since it won't affect our senders balance
                    continue;
                }
            }
        }

        // Now that we collected the transactions that were removed, we need to remove them from the mempool
        while let Some(tx) = txs_to_remove.pop() {
            mempool_state.transactions_by_fee.remove(&tx);
            mempool_state.transactions_by_age.remove(&tx);
        }

        // Iterate over the transactions in the reverted blocks,
        // what we need to know is if we need to add back the transaction into the mempool
        // This is similar to an operation where we try to add a transaction,
        // the only difference is that we don't need to re-check signature
        //
        for (_, block) in reverted_blocks {
            let transactions = block.transactions();

            if transactions.is_none() {
                continue;
            }

            for tx in transactions.unwrap().iter() {
                let block_height = blockchain.block_number() + 1;

                if !tx.is_valid_at(block_height)
                    || blockchain.contains_tx_in_validity_window(&tx.hash())
                {
                    // Tx has expired or is already included in the new chain, so skip it (TX is lost...)
                    continue;
                }

                let mut current_total = Coin::ZERO;

                if let Some(sender_state) = mempool_state.state_by_sender.get_mut(&tx.sender) {
                    if sender_state.txns.contains_key(&tx.hash()) {
                        // We already know this transaction, no need to process
                        continue;
                    }
                    current_total = sender_state.total;
                }

                let blockchain_balance: Coin;
                // Read the balance for this sender in the blockchain
                if let Some(sender_account) = blockchain.get_account(&tx.sender) {
                    blockchain_balance = sender_account.balance();
                } else {
                    // No sender in the blockchain for this tx, no need to process.
                    continue;
                }

                // Calculate the new balance assuming we add this transaction to the mempool
                let in_fly_balance = tx.total_value().unwrap() + current_total;

                if in_fly_balance <= blockchain_balance {
                    log::debug!(" Accepting new transaction from reverted blocks ");

                    if let Some(sender_state) = mempool_state.state_by_sender.get_mut(&tx.sender) {
                        sender_state.total = in_fly_balance;
                        sender_state.txns.insert(tx.hash(), tx.clone());
                    } else {
                        mempool_state.state_by_sender.insert(
                            tx.sender.clone(),
                            SenderPendingState {
                                total: in_fly_balance,
                                txns: HashMap::new(),
                            },
                        );
                        let sender_state =
                            mempool_state.state_by_sender.get_mut(&tx.sender).unwrap();
                        sender_state.txns.insert(tx.hash(), tx.clone());
                    }

                    // Add transaction to the mempool
                    mempool_state.transactions_by_fee.push(
                        tx.clone(),
                        TransactionFeePerByteOrdering {
                            transaction: Arc::new(tx.clone()),
                        },
                    );
                    mempool_state.transactions_by_age.push(
                        tx.clone(),
                        TransactionAgeOrdering {
                            transaction: Arc::new(tx.clone()),
                        },
                    );
                } else {
                    log::debug!(
                        "The Tx from reverted blocks was dropped because of not enough funds"
                    );
                }
            }
        }
    }
}

struct MemPoolState {
    // Transactions ordered by fee (higher fee transactions pop first)
    transactions_by_fee: KeyedPriorityQueue<Transaction, TransactionFeePerByteOrdering>,

    // Transactions ordered by age (older transactions pop first)
    // TODO: This could be improved to store a reference to the transaction and not the transaction itself
    transactions_by_age: KeyedPriorityQueue<Transaction, TransactionAgeOrdering>,

    // The in-fly balance per sender
    state_by_sender: HashMap<Address, SenderPendingState>,
}

struct SenderPendingState {
    // The Sum of the txns that are currently stored in the mempool for this sender
    total: Coin,

    // Transactions for this sender, we use transaction hash as key
    txns: HashMap<Blake2bHash, Transaction>,
}

struct MempoolExecutor<N: Network> {
    // Ongoing verification tasks counter
    verification_tasks: Arc<AtomicU32>,

    // Blockchain reference
    blockchain: Arc<RwLock<Blockchain>>,

    // The mempool state: the data structure where the transactions are stored
    state: Arc<RwLock<MemPoolState>>,

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
        state: Arc<RwLock<MemPoolState>>,
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
        state: Arc<RwLock<MemPoolState>>,
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
            let blockchain = self.blockchain.clone();
            let tasks_count = self.verification_tasks.clone();
            let mempool_state = self.state.clone();

            // Check if we already know the transaction
            {
                let state = mempool_state.read();

                if let Some(sender_state) = state.state_by_sender.get(&tx.sender) {
                    // Check if we already know this transaction
                    if sender_state.txns.contains_key(&tx.hash()) {
                        //We already know this transaction, no need to process
                        log::debug!("Transaction is already known ");
                        return Poll::Pending;
                    }
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
                // 1. Verify signature in a new blocking verification task, join handle after step 5 below
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
                // Check if transaction has already been mined.
                if blockchain.contains_tx_in_validity_window(&tx.hash()) {
                    log::debug!("Transaction has already been mined");
                    tasks_count.fetch_sub(1, AtomicOrdering::SeqCst);
                    return ReturnCode::Invalid;
                }
                // 4. Acquire MemPool upgrade -> write lock

                // 5. Sequencialize per Sender to Check Balances and acquire the upgradable from the blockchain),
                // Perform all balances checks
                let sender_account: Account;

                if let Some(account) = blockchain.get_account(&tx.sender) {
                    sender_account = account;
                } else {
                    log::debug!(
                        "There is no account for this sender in the blockchain {}",
                        tx.sender.to_user_friendly_address()
                    );
                    return ReturnCode::Invalid;
                }

                let blockchain_balance = sender_account.balance();

                // Read the pending transactions balance
                let mut current_balance = Coin::ZERO;

                let mut mempool_state = mempool_state.write();

                if let Some(sender_state) = mempool_state.state_by_sender.get_mut(&tx.sender) {
                    current_balance = sender_state.total;
                }
                // Calculate the new balance assuming we add this transaction to the mempool
                let in_fly_balance = tx.total_value().unwrap() + current_balance;

                if in_fly_balance <= blockchain_balance {
                    log::debug!(" Accepting new transaction");

                    if let Some(sender_state) = mempool_state.state_by_sender.get_mut(&tx.sender) {
                        sender_state.total = in_fly_balance;
                        sender_state.txns.insert(tx.hash(), tx.clone());
                    } else {
                        mempool_state.state_by_sender.insert(
                            tx.sender.clone(),
                            SenderPendingState {
                                total: in_fly_balance,
                                txns: HashMap::new(),
                            },
                        );
                        let sender_state =
                            mempool_state.state_by_sender.get_mut(&tx.sender).unwrap();
                        sender_state.txns.insert(tx.hash(), tx.clone());

                        log::debug!(
                            "Creating new state for sender {} ",
                            tx.sender.to_user_friendly_address()
                        );
                    }

                    // Now we update the mempool such that the per sender balance and what we have stored in the mempool is in sync
                    // 6. Add transaction to the mempool
                    mempool_state.transactions_by_fee.push(
                        tx.clone(),
                        TransactionFeePerByteOrdering {
                            transaction: Arc::new(tx.clone()),
                        },
                    );
                    mempool_state.transactions_by_age.push(
                        tx.clone(),
                        TransactionAgeOrdering {
                            transaction: Arc::new(tx),
                        },
                    );
                    // Mempool state upgraded is dropped
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

#[derive(Eq, PartialEq)]
struct TransactionFeePerByteOrdering {
    transaction: Arc<Transaction>,
}

impl Ord for TransactionFeePerByteOrdering {
    fn cmp(&self, other: &Self) -> Ordering {
        let this = self.transaction.as_ref();
        let other = other.transaction.as_ref();
        Ordering::Equal
            //.then_with(|| this.fee_per_byte().total_cmp(&other.fee_per_byte()))
            .then_with(|| this.fee.cmp(&other.fee))
            .then_with(|| this.value.cmp(&other.value))
            .then_with(|| this.recipient.cmp(&other.recipient))
            .then_with(|| this.validity_start_height.cmp(&other.validity_start_height))
            .then_with(|| this.sender.cmp(&other.sender))
            .then_with(|| this.recipient_type.cmp(&other.recipient_type))
            .then_with(|| this.sender_type.cmp(&other.sender_type))
            .then_with(|| this.flags.cmp(&other.flags))
            .then_with(|| this.data.len().cmp(&other.data.len()))
            .then_with(|| this.data.cmp(&other.data))
    }
}

impl PartialOrd for TransactionFeePerByteOrdering {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Eq, PartialEq)]
struct TransactionAgeOrdering {
    transaction: Arc<Transaction>,
}

impl Ord for TransactionAgeOrdering {
    fn cmp(&self, other: &Self) -> Ordering {
        let this = self.transaction.as_ref();
        let other = other.transaction.as_ref();
        Ordering::Equal
            .then_with(|| this.validity_start_height.cmp(&other.validity_start_height))
            //.then_with(|| this.fee_per_byte().total_cmp(&other.fee_per_byte()))
            .then_with(|| this.fee.cmp(&other.fee))
            .then_with(|| this.value.cmp(&other.value))
            .then_with(|| this.recipient.cmp(&other.recipient))
            .then_with(|| this.sender.cmp(&other.sender))
            .then_with(|| this.recipient_type.cmp(&other.recipient_type))
            .then_with(|| this.sender_type.cmp(&other.sender_type))
            .then_with(|| this.flags.cmp(&other.flags))
            .then_with(|| this.data.len().cmp(&other.data.len()))
            .then_with(|| this.data.cmp(&other.data))
    }
}

impl PartialOrd for TransactionAgeOrdering {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
