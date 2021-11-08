use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use futures::future::{AbortHandle, Abortable};
use futures::stream::BoxStream;
use keyed_priority_queue::KeyedPriorityQueue;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use beserial::Serialize;
use nimiq_block::Block;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_network_interface::network::Network;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::Transaction;

use crate::config::MempoolConfig;
use crate::executor::MempoolExecutor;
use crate::filter::{MempoolFilter, MempoolRules};
use crate::verify::{verify_tx, VerifyErr};

/// Struct defining the Mempool
pub struct Mempool {
    /// Blockchain reference
    pub blockchain: Arc<RwLock<Blockchain>>,

    /// The mempool state: the data structure where the transactions are stored
    pub(crate) state: Arc<RwLock<MempoolState>>,

    /// Mempool filter
    pub(crate) filter: Arc<RwLock<MempoolFilter>>,

    /// Mempool executor handle used to stop the executor
    pub(crate) executor_handle: Mutex<Option<AbortHandle>>,
}

impl Mempool {
    /// Creates a new mempool
    pub fn new(blockchain: Arc<RwLock<Blockchain>>, config: MempoolConfig) -> Self {
        let state = MempoolState {
            transactions: HashMap::new(),
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

    /// Starts the mempool executor
    ///
    /// Once this function is called, the mempool executor is spawned.
    /// The executor will subscribe to the transaction topic from the the network.
    pub async fn start_executor<N: Network>(&self, network: Arc<N>) {
        if self.executor_handle.lock().unwrap().is_some() {
            // If we already have an executor running, don't do anything
            return;
        }

        let mempool_executor = MempoolExecutor::new(
            Arc::clone(&self.blockchain),
            Arc::clone(&self.state),
            Arc::clone(&self.filter),
            Arc::clone(&network),
        )
        .await;

        // Start the executor and obtain its handle
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        tokio::spawn(Abortable::new(mempool_executor, abort_registration));

        // Set the executor handle
        *self.executor_handle.lock().unwrap() = Some(abort_handle);
    }

    /// Starts the mempool executor with a custom transaction stream
    ///
    /// Once this function is called, the mempool executor is spawned.
    /// The executor won't subscribe to the transaction topic from the network but will use the provided transaction
    /// stream instead.
    pub async fn start_executor_with_txn_stream<N: Network>(
        &self,
        txn_stream: BoxStream<'static, (Transaction, <N as Network>::PubsubId)>,
        network: Arc<N>,
    ) {
        if self.executor_handle.lock().unwrap().is_some() {
            // If we already have an executor running, don't do anything
            return;
        }

        let mempool_executor = MempoolExecutor::<N>::with_txn_stream(
            Arc::clone(&self.blockchain),
            Arc::clone(&self.state),
            Arc::clone(&self.filter),
            network,
            txn_stream,
        );

        // Start the executor and obtain its handle
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        tokio::spawn(Abortable::new(mempool_executor, abort_registration));

        // Set the executor handle
        *self.executor_handle.lock().unwrap() = Some(abort_handle);
    }

    /// Stops the mempool executor
    ///
    /// This functions should only be called only after one of the functions to start the executor is called.
    pub fn stop_executor(&self) {
        let mut handle = self.executor_handle.lock().unwrap();

        if handle.is_none() {
            // If there isn't any executor running we return
            return;
        }

        // Stop the executor
        handle.take().expect("Expected an executor handle").abort();
    }

    /// Updates the mempool given a set of reverted and adopted blocks.
    ///
    /// During a Blockchain extend event a new block is mined which implies that:
    ///
    /// 1. Existing transactions in the mempool can become invalidated because:
    ///     A. They are no longer valid at the new block height (aging)
    ///     B. Some were already mined
    ///
    /// 2. A transaction, that we didn't know about, from a known sender could be included in the blockchain, which implies:
    ///     A. We need to update the sender balances in our mempool because some txns in or mempool could become invalid
    ///
    /// 1.B and 2.A can be iterated over the txs in the adopted blocks, that is, it is not
    /// necessary to iterate all transactions in the mempool.
    ///
    pub fn mempool_update(
        &self,
        adopted_blocks: &[(Blake2bHash, Block)],
        reverted_blocks: &[(Blake2bHash, Block)],
    ) {
        // Acquire the mempool and blockchain locks
        let mut mempool_state = self.state.write();
        let blockchain = self.blockchain.read();

        let block_height = blockchain.block_number() + 1;

        // First remove the transactions that are no longer valid due to age.
        loop {
            // Get the hash of the oldest transaction.
            let tx_hash = match mempool_state.transactions_by_age.peek() {
                None => {
                    break;
                }
                Some((tx_hash, _)) => tx_hash.clone(),
            };

            // Get a reference to the transaction.
            let tx = mempool_state.get(&tx_hash).unwrap();

            // Check if it is still valid.
            if tx.is_valid_at(block_height) {
                // No need to process more transactions, since we arrived to the oldest one that is valid
                break;
            } else {
                // Remove the transaction from the mempool.
                mempool_state.remove(&tx_hash);
            }
        }

        // Now iterate over the transactions in the adopted blocks:
        //  if transaction was known:
        //    remove it from the mempool
        //  else
        //    if we know the sender
        //      update the sender state (some transactions could become invalid )
        //    else
        //      we don't care, since it won't affect our senders balance
        //
        for (_, block) in adopted_blocks {
            if let Some(transactions) = block.transactions() {
                for tx in transactions {
                    let tx_hash = tx.hash();

                    // Check if we already know this transaction. If yes, a known transaction was
                    // mined so we need to remove it from the mempool.
                    if mempool_state.contains(&tx_hash) {
                        mempool_state.remove(&tx_hash);
                        continue;
                    }

                    // Check if we know the sender of this transaction.
                    if let Some(sender_state) = mempool_state.state_by_sender.get_mut(&tx.sender) {
                        // This an unknown transaction from a known sender, we need to update our
                        // senders balance and some transactions could become invalid

                        // Lets read the balance from the blockchain
                        let sender_balance = blockchain.get_account(&tx.sender).unwrap().balance();

                        // Check if the sender still has enough funds to pay for all pending
                        // transactions.
                        if sender_state.total > sender_balance {
                            // If not, we remove transactions until he is able to pay.
                            let mut new_total = Coin::ZERO;

                            // TODO: We could have per sender transactions ordered by fee to try to
                            //       keep the ones with higher fee
                            let sender_txs: Vec<Blake2bHash> =
                                sender_state.txns.iter().cloned().collect();

                            let txs_to_remove: Vec<&Blake2bHash> = sender_txs
                                .iter()
                                .filter(|hash| {
                                    let old_tx = mempool_state.get(hash).unwrap();

                                    if old_tx.total_value().unwrap() + new_total <= sender_balance {
                                        new_total += old_tx.total_value().unwrap();
                                        false
                                    } else {
                                        true
                                    }
                                })
                                .collect();

                            for hash in txs_to_remove {
                                mempool_state.remove(hash);
                            }
                        }
                    }
                }
            }
        }

        // Iterate over the transactions in the reverted blocks,
        // what we need to know is if we need to add back the transaction into the mempool
        // This is similar to an operation where we try to add a transaction,
        // the only difference is that we don't need to re-check signature
        for (_, block) in reverted_blocks {
            let block_height = blockchain.block_number() + 1;

            if let Some(transactions) = block.transactions() {
                for tx in transactions {
                    let tx_hash = tx.hash();

                    // Check if we already know this transaction. If yes, skip ahead.
                    if mempool_state.contains(&tx_hash) {
                        continue;
                    }

                    // Check if transaction is still valid.
                    if !tx.is_valid_at(block_height)
                        || blockchain.contains_tx_in_validity_window(&tx_hash, None)
                    {
                        // Tx has expired or is already included in the new chain, so skip it
                        // (TX is lost...)
                        continue;
                    }

                    // Get the sender's account balance.
                    let sender_balance = match blockchain.get_account(&tx.sender) {
                        None => {
                            // No sender in the blockchain for this tx, no need to process.
                            continue;
                        }
                        Some(sender_account) => sender_account.balance(),
                    };

                    // Get the sender's transaction total.
                    let sender_total = match mempool_state.state_by_sender.get(&tx.sender) {
                        None => Coin::ZERO,
                        Some(sender_state) => sender_state.total,
                    };

                    // Calculate the new balance assuming we add this transaction to the mempool
                    let in_fly_balance = tx.total_value().unwrap() + sender_total;

                    if in_fly_balance <= sender_balance {
                        log::debug!("Accepting new transaction from reverted blocks");
                        mempool_state.put(tx);
                    } else {
                        log::debug!(
                            "The Tx from reverted blocks was dropped because of not enough funds"
                        );
                    }
                }
            }
        }
    }

    /// Returns a vector with accepted transactions from the mempool.
    ///
    /// Returns the highest fee per byte up to max_bytes transactions and removes them from the mempool
    pub fn get_transactions_for_block(&self, max_bytes: usize) -> Vec<Transaction> {
        let mut tx_vec = vec![];

        let state = self.state.upgradable_read();

        if state.transactions.is_empty() {
            log::debug!("Requesting txns and there are no txns in the mempool ");
            return tx_vec;
        }

        let mut size = 0_usize;

        let mut mempool_state_upgraded = RwLockUpgradableReadGuard::upgrade(state);

        loop {
            // Get the hash of the highest paying transaction.
            let tx_hash = match mempool_state_upgraded.transactions_by_fee.peek() {
                None => {
                    break;
                }
                Some((tx_hash, _)) => tx_hash.clone(),
            };

            // Get the transaction.
            let tx = mempool_state_upgraded.get(&tx_hash).unwrap().clone();

            // Calculate size. If we can't fit the transaction in the block, then we stop here.
            // TODO: We can optimize this. There might be a smaller transaction that still fits.
            size += tx.serialized_size();

            if size > max_bytes {
                break;
            }

            // Remove the transaction from the mempool.
            mempool_state_upgraded.remove(&tx_hash);

            // Push the transaction to our output vector.
            tx_vec.push(tx);
        }

        log::debug!(
            "Returning {} transactions from mempool ({} remaining)",
            tx_vec.len(),
            mempool_state_upgraded.transactions.len()
        );

        tx_vec
    }

    /// Adds a transaction to the Mempool.
    pub async fn add_transaction(&self, transaction: Transaction) -> Result<(), VerifyErr> {
        let blockchain = Arc::clone(&self.blockchain);
        let mempool_state = Arc::clone(&self.state);
        let filter = Arc::clone(&self.filter);

        let verify_tx_ret = verify_tx(&transaction, blockchain, &mempool_state, filter).await;

        match verify_tx_ret {
            Ok(mempool_state_lock) => {
                RwLockUpgradableReadGuard::upgrade(mempool_state_lock).put(&transaction);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Checks whether a transaction has been filtered
    pub fn is_filtered(&self, hash: &Blake2bHash) -> bool {
        self.filter.read().blacklisted(hash)
    }

    /// Returns the rules for the mempool.
    pub fn get_rules(&self) -> MempoolRules {
        self.filter.read().rules.clone()
    }

    /// Checks if a transactions is in the mempool, by its hash.
    pub fn contains_transaction_by_hash(&self, hash: &Blake2bHash) -> bool {
        self.state.read().contains(hash)
    }

    /// Gets a transactions by its hash.
    pub fn get_transaction_by_hash(&self, hash: &Blake2bHash) -> Option<Transaction> {
        self.state.read().get(hash).cloned()
    }

    /// Gets all transaction hashes in the mempool.
    pub fn get_transaction_hashes(&self) -> Vec<Blake2bHash> {
        self.state.read().transactions.keys().cloned().collect()
    }

    /// Gets all transactions in the mempool.
    pub fn get_transactions(&self) -> Vec<Transaction> {
        self.state.read().transactions.values().cloned().collect()
    }
}

pub(crate) struct MempoolState {
    // A hashmap containing the transactions indexed by their hash.
    pub(crate) transactions: HashMap<Blake2bHash, Transaction>,

    // Transactions ordered by fee (higher fee transactions pop first)
    pub(crate) transactions_by_fee: KeyedPriorityQueue<Blake2bHash, FeeWrapper>,

    // Transactions ordered by age (older transactions pop first)
    pub(crate) transactions_by_age: KeyedPriorityQueue<Blake2bHash, u32>,

    // The in-fly balance per sender
    pub(crate) state_by_sender: HashMap<Address, SenderPendingState>,
}

impl MempoolState {
    pub fn contains(&self, hash: &Blake2bHash) -> bool {
        self.transactions.contains_key(hash)
    }

    pub fn get(&self, hash: &Blake2bHash) -> Option<&Transaction> {
        self.transactions.get(hash)
    }

    pub(crate) fn put(&mut self, tx: &Transaction) -> bool {
        let tx_hash = tx.hash();

        if self.transactions.contains_key(&tx_hash) {
            return false;
        }

        self.transactions.insert(tx_hash.clone(), tx.clone());

        self.transactions_by_fee
            .push(tx_hash.clone(), FeeWrapper(tx.fee_per_byte()));

        self.transactions_by_age
            .push(tx_hash.clone(), tx.validity_start_height);

        match self.state_by_sender.get_mut(&tx.sender) {
            None => {
                let mut txns = HashSet::new();
                txns.insert(tx_hash);

                self.state_by_sender.insert(
                    tx.sender.clone(),
                    SenderPendingState {
                        total: tx.total_value().unwrap(),
                        txns,
                    },
                );
            }
            Some(sender_state) => {
                sender_state.total += tx.total_value().unwrap();
                sender_state.txns.insert(tx_hash);
            }
        }

        true
    }

    pub(crate) fn remove(&mut self, tx_hash: &Blake2bHash) -> Option<Transaction> {
        let tx = self.transactions.remove(tx_hash)?;

        self.transactions_by_age.remove(tx_hash);
        self.transactions_by_fee.remove(tx_hash);

        let sender_state = self.state_by_sender.get_mut(&tx.sender).unwrap();

        sender_state.total -= tx.total_value().unwrap();
        sender_state.txns.remove(tx_hash);

        if sender_state.txns.is_empty() {
            self.state_by_sender.remove(&tx.sender);
        }

        Some(tx)
    }
}

pub(crate) struct SenderPendingState {
    // The sum of the txns that are currently stored in the mempool for this sender
    pub(crate) total: Coin,

    // Transaction hashes for this sender.
    pub(crate) txns: HashSet<Blake2bHash>,
}

/// Since f64 doesn't implement Ord, we cannot sort f64's or use them in KeyedPriorityQueues. So we
/// create this wrapper and implement Ord ourselves.
// TODO: Maybe use this wrapper to do more fine ordering. For example, we might prefer small size
//       transactions over large size transactions (assuming they have the same fee per byte). Or
//       we might prefer basic transactions over staking contract transactions, etc, etc.
#[derive(PartialEq)]
pub struct FeeWrapper(f64);

impl Eq for FeeWrapper {}

impl PartialOrd for FeeWrapper {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FeeWrapper {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}
