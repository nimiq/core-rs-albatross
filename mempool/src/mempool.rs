use std::{
    collections::HashSet,
    sync::{atomic::AtomicU32, Arc},
};

use futures::{
    future::{AbortHandle, Abortable},
    lock::{Mutex, MutexGuard},
    stream::{select, BoxStream, StreamExt},
};
use nimiq_account::ReservedBalance;
use nimiq_block::Block;
use nimiq_blockchain::{Blockchain, TransactionVerificationCache};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_network_interface::network::{Network, Topic};
use nimiq_serde::Serialize;
use nimiq_transaction::{
    historic_transaction::RawTransactionHash, ControlTransactionTopic, Transaction,
    TransactionTopic,
};
use nimiq_utils::spawn;
use parking_lot::RwLock;
use tokio_metrics::TaskMonitor;

#[cfg(feature = "metrics")]
use crate::mempool_metrics::MempoolMetrics;
use crate::{
    config::MempoolConfig,
    executor::{MempoolExecutor, PubsubIdOrPeerId},
    filter::{MempoolFilter, MempoolRules},
    mempool_state::{EvictionReason, MempoolState},
    mempool_transactions::{MempoolTransactions, TxPriority},
    sync::{messages::MempoolTransactionType, MempoolSyncer},
    verify::{verify_tx, VerifyErr},
};

/// Struct defining the Mempool
pub struct Mempool {
    /// Blockchain reference
    blockchain: Arc<RwLock<Blockchain>>,

    /// The mempool state: the data structure where the transactions are stored
    pub(crate) state: Arc<RwLock<MempoolState>>,

    /// Mempool filter
    pub(crate) filter: Arc<RwLock<MempoolFilter>>,

    /// Mempool executor handle used to stop the executor
    pub(crate) executor_handle: Mutex<Option<AbortHandle>>,

    /// Mempool executor handle used to stop the control mempool executor
    pub(crate) control_executor_handle: Mutex<Option<AbortHandle>>,

    /// Total number of ongoing verification tasks
    verification_tasks: Arc<AtomicU32>,
}

impl Mempool {
    /// Default total size limit of transactions in the mempool (bytes)
    pub const DEFAULT_SIZE_LIMIT: usize = 12_000_000;

    /// Default total size limit of control transactions in the mempool (bytes)
    pub const DEFAULT_CONTROL_SIZE_LIMIT: usize = 6_000_000;

    /// Creates a new mempool
    pub fn new(blockchain: Arc<RwLock<Blockchain>>, config: MempoolConfig) -> Self {
        let state = Arc::new(RwLock::new(MempoolState::new(
            config.size_limit,
            config.control_size_limit,
        )));

        Self {
            blockchain,
            state: Arc::clone(&state),
            filter: Arc::new(RwLock::new(MempoolFilter::new(
                config.filter_rules,
                config.filter_limit,
            ))),
            executor_handle: Mutex::new(None),
            control_executor_handle: Mutex::new(None),
            verification_tasks: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Start the `MempoolExecutor` for `Topic` `T` and instrument a monitor for the task if given.
    /// An `AbortHandle` will be stored in `handle`.
    fn start_executor<N: Network, T: Topic + Unpin + Send + Sync + 'static>(
        &self,
        network: Arc<N>,
        monitor: Option<TaskMonitor>,
        mut handle: MutexGuard<'_, Option<AbortHandle>>,
        txn_stream: BoxStream<'static, (Transaction, PubsubIdOrPeerId<N>)>,
    ) {
        if handle.is_some() {
            // If we already have an executor running, don't do anything
            return;
        }

        // Create the executor for the Topic T
        let executor = MempoolExecutor::<N, T>::new(
            Arc::clone(&self.blockchain),
            Arc::clone(&self.state),
            Arc::clone(&self.filter),
            Arc::clone(&network),
            txn_stream,
            Arc::clone(&self.verification_tasks),
        );

        // Create the AbortHandle
        let (abort_handle, abort_registration) = AbortHandle::new_pair();

        // Create the abortable future using the executor and the abort registration
        let future = async move {
            let _ = Abortable::new(executor, abort_registration).await;
        };

        // if a monitor was given, instrument the spawned task
        if let Some(monitor) = monitor {
            spawn(monitor.instrument(future));
        } else {
            spawn(future);
        }

        // Set the executor handle
        *handle = Some(abort_handle);
    }

    /// Starts the mempool executors
    ///
    /// Once this function is called, the mempool executors are spawned.
    /// Both the regular and control txn executors are subscribed to their respective txn topics.
    pub async fn start_executors<N: Network>(
        &self,
        network: Arc<N>,
        monitor: Option<TaskMonitor>,
        control_monitor: Option<TaskMonitor>,
    ) {
        let executor_handle = self.executor_handle.lock().await;
        let control_executor_handle = self.control_executor_handle.lock().await;

        if executor_handle.is_some() && control_executor_handle.is_some() {
            // If we already have both executors running dont do anything
            return;
        }

        // TODO: get correct peers
        // TODO: only get peers that are synced with us
        // Sync regular transactions with the mempool of other peers
        let regular_transactions_syncer = MempoolSyncer::new(
            network.get_peers(),
            MempoolTransactionType::Regular,
            Arc::clone(&network),
            Arc::clone(&self.blockchain),
            Arc::clone(&self.state),
        );

        // Subscribe to the network TX topic
        let txn_stream = network
            .subscribe::<TransactionTopic>()
            .await
            .unwrap()
            .map(|(tx, pubsub_id)| (tx, PubsubIdOrPeerId::PubsubId(pubsub_id)))
            .boxed();

        self.start_executor::<N, TransactionTopic>(
            Arc::clone(&network),
            monitor,
            executor_handle,
            select(regular_transactions_syncer, txn_stream).boxed(),
        );

        // TODO: get correct peers
        // TODO: only get peers that are synced with us
        // Sync control transactions with the mempool of other peers
        let control_transactions_syncer = MempoolSyncer::new(
            network.get_peers(),
            MempoolTransactionType::Control,
            Arc::clone(&network),
            Arc::clone(&self.blockchain),
            Arc::clone(&self.state),
        );

        // Subscribe to the control transaction topic
        let txn_stream = network
            .subscribe::<ControlTransactionTopic>()
            .await
            .unwrap()
            .map(|(tx, pubsub_id)| (Transaction::from(tx), PubsubIdOrPeerId::PubsubId(pubsub_id)))
            .boxed();

        self.start_executor::<N, ControlTransactionTopic>(
            network,
            control_monitor,
            control_executor_handle,
            select(control_transactions_syncer, txn_stream).boxed(),
        );
    }

    /// Starts the mempool executor with a custom transaction stream
    ///
    /// Once this function is called, the mempool executor is spawned.
    /// The executor won't subscribe to the transaction topic from the network but will use the provided transaction
    /// stream instead.
    pub async fn start_executor_with_txn_stream<N: Network>(
        &self,
        txn_stream: BoxStream<'static, (Transaction, PubsubIdOrPeerId<N>)>,
        network: Arc<N>,
    ) {
        self.start_executor::<N, TransactionTopic>(
            network,
            None,
            self.executor_handle.lock().await,
            txn_stream,
        )
    }

    /// Starts the control mempool executor with a custom transaction stream
    ///
    /// Once this function is called, the mempool executor is spawned.
    /// The executor won't subscribe to the transaction topic from the network but will use the provided transaction
    /// stream instead.
    pub async fn start_control_executor_with_txn_stream<N: Network>(
        &self,
        txn_stream: BoxStream<'static, (Transaction, PubsubIdOrPeerId<N>)>,
        network: Arc<N>,
    ) {
        self.start_executor::<N, ControlTransactionTopic>(
            network,
            None,
            self.control_executor_handle.lock().await,
            txn_stream,
        )
    }

    /// Stops the mempool executors
    ///
    /// This functions should only be called only after one of the functions to start the executors is called.
    pub async fn stop_executors<N: Network>(&self, network: Arc<N>) {
        let mut handle = self.executor_handle.lock().await;
        let mut control_executor_handle = self.control_executor_handle.lock().await;

        if handle.is_none() && control_executor_handle.is_none() {
            // If there isn't any executors running we return
            return;
        }

        // Unsubscribe to the network TX topic before killing the executor
        network.unsubscribe::<TransactionTopic>().await.unwrap();

        // Stop the executor
        handle.take().expect("Expected an executor handle").abort();

        // Unsubscribe to the network control TX topic before killing the executor
        network
            .unsubscribe::<ControlTransactionTopic>()
            .await
            .unwrap();

        // Stop the control executor
        control_executor_handle
            .take()
            .expect("Expected a control executor handle")
            .abort();
    }

    /// Stops the mempool executor without TX stream
    ///
    /// This function is used for testing purposes (along with the start_executor_with_txn_stream function)
    /// it is the responsibility of the caller to subscribe and unsubscribe from the topic accordingly
    ///
    /// This functions should only be called only after one of the functions to start the executor is called.
    pub async fn stop_executor_without_unsubscribe(&self) {
        let mut handle = self.executor_handle.lock().await;

        if handle.is_none() {
            // If there isn't any executor running we return
            return;
        }

        // Stop the executor
        handle.take().expect("Expected an executor handle").abort();
    }

    /// Stops the control mempool executor without TX stream
    ///
    /// This function is used for testing purposes (along with the start_executor_with_txn_stream function)
    /// it is the responsibility of the caller to subscribe and unsubscribe from the topic accordingly
    ///
    /// This functions should only be called only after one of the functions to start the executor is called.
    pub async fn stop_control_executor_without_unsubscribe(&self) {
        let mut handle = self.control_executor_handle.lock().await;

        if handle.is_none() {
            // If there isn't any executor running we return
            return;
        }

        // Stop the executor
        handle
            .take()
            .expect("Expected a control executor handle")
            .abort();
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
    pub fn update(
        &self,
        adopted_blocks: &[(Blake2bHash, Block)],
        reverted_blocks: &[(Blake2bHash, Block)],
    ) {
        // Acquire the mempool and blockchain locks.
        let blockchain = self.blockchain.read();
        let mut mempool_state = self.state.write();

        // First remove the transactions that are no longer valid due to age.
        self.prune_expired_transactions(&blockchain, &mut mempool_state);

        // Now iterate over the transactions in the adopted blocks:
        //  if transaction was known:
        //    remove it from the mempool
        //  else
        //    if we know the sender
        //      update the sender state (some transactions could become invalid)
        //    else
        //      we don't care, since it won't affect our senders balance
        let mut affected_senders = HashSet::new();
        for (_, block) in adopted_blocks {
            if let Some(transactions) = block.transactions() {
                for tx in transactions {
                    let tx = tx.get_raw_transaction();
                    let tx_hash = tx.hash();

                    // Check if we already know this transaction. If yes, a known transaction was
                    // mined so we need to remove it from the mempool.
                    if mempool_state.contains(&tx_hash) {
                        mempool_state.remove(
                            &blockchain,
                            &tx_hash,
                            EvictionReason::AlreadyIncluded,
                        );
                        continue;
                    }

                    // Check if we know the sender of this transaction.
                    if mempool_state.state_by_sender.contains_key(&tx.sender) {
                        // This an unknown transaction from a known sender, we need to update our
                        // senders balance and some transactions could become invalid
                        affected_senders.insert(tx.sender.clone());
                    }
                }
            }
        }

        // Update all sender balances that were affected by the adopted blocks.
        // Remove the transactions that have become invalid.
        Mempool::recompute_sender_balances(affected_senders, &blockchain, &mut mempool_state);

        // Iterate over the transactions in the reverted blocks,
        // what we need to know is if we need to add back the transaction into the mempool
        // This is similar to an operation where we try to add a transaction,
        // the only difference is that we don't need to re-check signature
        for (_, block) in reverted_blocks {
            if let Some(transactions) = block.transactions() {
                for tx in transactions {
                    let tx = tx.get_raw_transaction();
                    let tx_hash = tx.hash();

                    // Check if we already know this transaction. If yes, skip ahead.
                    if mempool_state.contains(&tx_hash) {
                        continue;
                    }

                    // Check if transaction is still valid.
                    let next_block_number = blockchain.block_number() + 1;
                    if !tx.is_valid_at(next_block_number) {
                        continue;
                    }

                    // Check that the transaction has not already been included.
                    if blockchain.contains_tx_in_validity_window(&tx_hash.into(), None) {
                        continue;
                    }

                    // Add the transaction to the mempool. Balance checks are performed within put().
                    mempool_state
                        .put(&blockchain, tx.clone(), TxPriority::Medium)
                        .ok();
                }
            }
        }
    }

    /// Get the mempool into a consistent and up-to-date state.
    /// Needed after the consensus was lost and the mempool didn't receive any information during that time
    /// - Removes transactions that expired, that were included in a block already or for which the sender is lacking funds by now.
    /// - Recompute reserved balances.
    pub fn cleanup(&self) {
        let blockchain = self.blockchain.read();
        let mut mempool_state = self.state.write();

        self.prune_expired_transactions(&blockchain, &mut mempool_state);

        // Remove all transactions that have already been included.
        let regular_iter = mempool_state.regular_transactions.transactions.iter();
        let control_iter = mempool_state.control_transactions.transactions.iter();
        let included_txs = regular_iter
            .chain(control_iter)
            .map(|tx| tx.0)
            .filter(|tx_hash| {
                blockchain.contains_tx_in_validity_window(
                    &RawTransactionHash::from((*tx_hash).clone()),
                    None,
                )
            })
            .cloned()
            .collect::<Vec<Blake2bHash>>();

        for tx_hash in included_txs {
            mempool_state.remove(&blockchain, &tx_hash, EvictionReason::AlreadyIncluded);
        }

        // Recompute reserved balances, potentially removing transactions that have become invalid.
        let all_known_senders = mempool_state
            .state_by_sender
            .keys()
            .cloned()
            .collect::<HashSet<Address>>();

        Mempool::recompute_sender_balances(all_known_senders, &blockchain, &mut mempool_state);
    }

    // Update all balances of senders with `addresses`.
    // Remove the transactions that have become invalid.
    fn recompute_sender_balances(
        addresses: HashSet<Address>,
        blockchain: &Blockchain,
        mempool_state: &mut MempoolState,
    ) {
        for address in addresses {
            // The sender_state does not exist anymore if all transactions from this sender have
            // been mined.
            let mut sender_state = match mempool_state.state_by_sender.remove(&address) {
                Some(state) => state,
                None => continue,
            };
            sender_state.reserved_balance = ReservedBalance::new(address.clone());

            let sender_account = match blockchain.get_account_if_complete(&address) {
                Some(account) => account,
                None => {
                    // We don't have the sender account so we can't do any balance tracking.
                    // Remove all transactions from this sender.
                    for hash in &sender_state.txns {
                        mempool_state
                            .regular_transactions
                            .delete(hash)
                            .or_else(|| mempool_state.control_transactions.delete(hash));
                    }
                    continue;
                }
            };

            // TODO We should have per sender transactions ordered by fee to try to
            //  keep the ones with higher fee

            sender_state.txns.retain(|tx_hash| {
                let tx = match mempool_state.get(tx_hash) {
                    Some(transaction) => transaction,
                    None => return false,
                };
                let still_valid = blockchain
                    .reserve_balance(&sender_account, tx, &mut sender_state.reserved_balance)
                    .is_ok();
                if !still_valid {
                    mempool_state.remove(blockchain, tx_hash, EvictionReason::Invalid);
                }
                still_valid
            });

            if !sender_state.txns.is_empty() {
                mempool_state.state_by_sender.insert(address, sender_state);
            }
        }
    }

    /// Remove transactions that are expired and thus no longer valid.
    fn prune_expired_transactions(
        &self,
        blockchain: &Blockchain,
        mempool_state: &mut MempoolState,
    ) {
        let next_block_number = blockchain.block_number() + 1;
        let expired_txns = mempool_state.get_expired_txns(next_block_number);
        for tx_hash in expired_txns {
            mempool_state.remove(blockchain, &tx_hash, EvictionReason::Expired);
        }
    }

    /// Returns a vector with accepted transactions from the mempool.
    /// Note that this takes a read lock on blockchain.
    ///
    /// Returns the highest fee per byte up to max_bytes transactions and removes them from the mempool.
    /// It also return the sum of the serialized size of the returned transactions.
    pub fn get_transactions_for_block(&self, max_bytes: usize) -> (Vec<Transaction>, usize) {
        let blockchain = self.blockchain.read();
        self.get_transactions_for_block_locked(&blockchain, max_bytes)
    }

    /// Returns a vector with accepted transactions from the mempool.
    /// If the caller already holds a blockchain lock, it can be passed to this function to prevent
    /// double-locking the blockchain.
    ///
    /// Returns the highest fee per byte up to max_bytes transactions and removes them from the mempool.
    /// It also return the sum of the serialized size of the returned transactions.
    pub fn get_transactions_for_block_locked(
        &self,
        blockchain: &Blockchain,
        max_bytes: usize,
    ) -> (Vec<Transaction>, usize) {
        let mut state = self.state.write();
        let (txs, size) =
            Self::get_transactions_for_block_impl(&mut state.regular_transactions, max_bytes);

        for tx in &txs {
            state.remove(blockchain, &tx.hash(), EvictionReason::BlockBuilding);
        }

        debug!(
            returned_txs = txs.len(),
            remaining_txs = state.regular_transactions.len(),
            "Returned regular transactions from mempool"
        );

        (txs, size)
    }

    /// Returns a vector with accepted control transactions from the mempool.
    /// Note that this takes a read lock on blockchain.
    ///
    /// Returns the highest fee per byte up to max_bytes transactions and removes them from the mempool.
    /// It also return the sum of the serialized size of the returned transactions.
    pub fn get_control_transactions_for_block(
        &self,
        max_bytes: usize,
    ) -> (Vec<Transaction>, usize) {
        let blockchain = self.blockchain.read();
        self.get_control_transactions_for_block_locked(&blockchain, max_bytes)
    }

    /// Returns a vector with accepted control transactions from the mempool.
    /// If the caller already holds a blockchain lock, it can be passed to this function to prevent
    /// double-locking the blockchain.
    ///
    /// Returns the highest fee per byte up to max_bytes transactions and removes them from the mempool.
    /// It also return the sum of the serialized size of the returned transactions.
    pub fn get_control_transactions_for_block_locked(
        &self,
        blockchain: &Blockchain,
        max_bytes: usize,
    ) -> (Vec<Transaction>, usize) {
        let mut state = self.state.write();
        let (txs, size) =
            Self::get_transactions_for_block_impl(&mut state.control_transactions, max_bytes);

        for tx in &txs {
            state.remove(blockchain, &tx.hash(), EvictionReason::BlockBuilding);
        }

        debug!(
            returned_txs = txs.len(),
            remaining_txs = state.control_transactions.len(),
            "Returned control transactions from mempool"
        );

        (txs, size)
    }

    fn get_transactions_for_block_impl(
        transactions: &mut MempoolTransactions,
        max_bytes: usize,
    ) -> (Vec<Transaction>, usize) {
        let mut txs = vec![];
        let mut size = 0_usize;

        loop {
            // Get the hash of the highest paying transactions.
            let tx_hash = match transactions.best_transactions.peek() {
                None => break,
                Some((tx_hash, _)) => tx_hash.clone(),
            };

            // Get the transaction.
            let tx = transactions.get(&tx_hash).unwrap().clone();

            // Calculate size. If we can't fit the transaction in the block, then we stop here.
            // TODO: We can optimize this. There might be a smaller transaction that still fits.
            // We need to account for one extra byte per transaction to encode its final execution status
            let next_size = size + 1 + tx.serialized_size();
            if next_size > max_bytes {
                break;
            }
            size = next_size;

            // Remove the transaction from best_transactions so that we can advance.
            // The caller needs to clean up the rest of the data structures.
            transactions.best_transactions.pop();

            // Push the transaction to our output vector.
            txs.push(tx);
        }

        (txs, size)
    }

    /// Adds a transaction to the Mempool.
    pub fn add_transaction(
        &self,
        transaction: Transaction,
        tx_priority: Option<TxPriority>,
    ) -> Result<(), VerifyErr> {
        let blockchain = Arc::clone(&self.blockchain);
        let mempool_state = Arc::clone(&self.state);
        let filter = Arc::clone(&self.filter);
        let network_id = blockchain.read().network_id;
        verify_tx(
            transaction,
            blockchain,
            network_id,
            &mempool_state,
            filter,
            tx_priority.unwrap_or(TxPriority::Medium),
        )
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
        let state = self.state.read();

        state
            .regular_transactions
            .transactions
            .keys()
            .cloned()
            .chain(state.control_transactions.transactions.keys().cloned())
            .collect()
    }

    /// Returns the number of pending transactions in mempool.
    pub fn num_transactions(&self) -> usize {
        let state = self.state.read();
        state.regular_transactions.transactions.len()
            + state.control_transactions.transactions.len()
    }

    /// Gets all transactions in the mempool (control txns come first)
    pub fn get_transactions(&self) -> Vec<Transaction> {
        let state = self.state.read();

        state
            .control_transactions
            .transactions
            .values()
            .cloned()
            .chain(state.regular_transactions.transactions.values().cloned())
            .collect()
    }

    /// Returns the current metrics
    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> Arc<MempoolMetrics> {
        self.state.read().metrics.clone()
    }
}

impl TransactionVerificationCache for Mempool {
    fn is_known(&self, tx_hash: &Blake2bHash) -> bool {
        if let Some(state) = self.state.try_read() {
            state.contains(tx_hash)
        } else {
            false
        }
    }
}
