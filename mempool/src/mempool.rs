use futures::future::{AbortHandle, Abortable};
use futures::lock::Mutex;
use futures::stream::BoxStream;
use keyed_priority_queue::KeyedPriorityQueue;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use beserial::Serialize;
use nimiq_account::{Account, BasicAccount};
use nimiq_block::Block;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, TransactionVerificationCache};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_network_interface::network::{Network, Topic};
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};
use nimiq_transaction::Transaction;

use crate::config::MempoolConfig;
use crate::executor::MempoolExecutor;
use crate::filter::{MempoolFilter, MempoolRules};
use crate::verify::{verify_tx, VerifyErr};

/// Transaction topic for the Mempool to request transactions from the network
#[derive(Clone, Debug, Default)]
pub struct TransactionTopic;

impl Topic for TransactionTopic {
    type Item = Transaction;

    const BUFFER_SIZE: usize = 1024;
    const NAME: &'static str = "transactions";
    const VALIDATE: bool = true;
}

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
    /// Default total size limit of transactions in the mempool (bytes)
    pub const DEFAULT_SIZE_LIMIT: usize = 12000000;

    /// Creates a new mempool
    pub fn new(blockchain: Arc<RwLock<Blockchain>>, config: MempoolConfig) -> Self {
        let state = MempoolState {
            transactions: HashMap::new(),
            best_transactions: KeyedPriorityQueue::new(),
            worst_transactions: KeyedPriorityQueue::new(),
            oldest_transactions: KeyedPriorityQueue::new(),
            state_by_sender: HashMap::new(),
            outgoing_validators: HashMap::new(),
            outgoing_stakers: HashMap::new(),
            creating_validators: HashMap::new(),
            creating_stakers: HashMap::new(),
            total_size_limit: config.size_limit,
            total_size: 0,
            tx_counter: 0,
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
        let mut executor_handle = self.executor_handle.lock().await;

        if executor_handle.is_some() {
            // If we already have an executor running, don't do anything
            return;
        }

        // Subscribe to the network TX topic
        let txn_stream = network.subscribe::<TransactionTopic>().await.unwrap();

        let mempool_executor = MempoolExecutor::new(
            Arc::clone(&self.blockchain),
            Arc::clone(&self.state),
            Arc::clone(&self.filter),
            Arc::clone(&network),
            txn_stream,
        );

        // Start the executor and obtain its handle
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        tokio::spawn(Abortable::new(mempool_executor, abort_registration));

        // Set the executor handle
        *executor_handle = Some(abort_handle);
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
        let mut executor_handle = self.executor_handle.lock().await;

        if executor_handle.is_some() {
            // If we already have an executor running, don't do anything
            return;
        }

        let mempool_executor = MempoolExecutor::<N>::new(
            Arc::clone(&self.blockchain),
            Arc::clone(&self.state),
            Arc::clone(&self.filter),
            Arc::clone(&network),
            txn_stream,
        );

        // Start the executor and obtain its handle
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        tokio::spawn(Abortable::new(mempool_executor, abort_registration));

        // Set the executor handle
        *executor_handle = Some(abort_handle);
    }

    /// Stops the mempool executor
    ///
    /// This functions should only be called only after one of the functions to start the executor is called.
    pub async fn stop_executor<N: Network>(&self, network: Arc<N>) {
        let mut handle = self.executor_handle.lock().await;

        if handle.is_none() {
            // If there isn't any executor running we return
            return;
        }

        // Unsubscribe to the network TX topic before killing the executor
        network.unsubscribe::<TransactionTopic>().await.unwrap();

        // Stop the executor
        handle.take().expect("Expected an executor handle").abort();
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
        let blockchain = self.blockchain.read();
        let mut mempool_state = self.state.write();

        let block_height = blockchain.block_number() + 1;

        // First remove the transactions that are no longer valid due to age.
        loop {
            // Get the hash of the oldest transaction.
            let tx_hash = match mempool_state.oldest_transactions.peek() {
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
                    if let Some(sender_state) = mempool_state.state_by_sender.get(&tx.sender) {
                        // This an unknown transaction from a known sender, we need to update our
                        // senders balance and some transactions could become invalid

                        //Obtain the sender account. Signaling txns from adopted blocks should be allowed
                        let sender_account =
                            match blockchain.get_account(&tx.sender).or_else(|| {
                                if tx.total_value() != Coin::ZERO {
                                    None
                                } else {
                                    Some(Account::Basic(BasicAccount {
                                        balance: Coin::ZERO,
                                    }))
                                }
                            }) {
                                None => {
                                    log::debug!(
                                        "There is no account for this sender in the blockchain {}",
                                        tx.sender.to_user_friendly_address()
                                    );
                                    // The account from this sender was pruned/removed, so we need to delete all txns sent from this address
                                    mempool_state.remove_sender_txns(&tx.sender);
                                    continue;
                                }
                                Some(account) => account,
                            };

                        let sender_balance = sender_account.balance();

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

                                    if old_tx.total_value() + new_total <= sender_balance {
                                        new_total += old_tx.total_value();
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

                    // Perform checks for staking transactions
                    let mut txs_to_remove: Vec<Blake2bHash> = vec![];
                    // If it is an outgoing staking transaction then we have additional checks.
                    if tx.sender_type == AccountType::Staking {
                        // Parse transaction data.
                        let data = OutgoingStakingTransactionProof::parse(tx)
                            .expect("The proof should have already been parsed");

                        // If the sender was already in the mempool then we need to remove the transaction.
                        let transaction = match data.clone() {
                            OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                                mempool_state
                                    .outgoing_validators
                                    .get(&proof.compute_signer())
                            }
                            OutgoingStakingTransactionProof::Unstake { proof } => {
                                mempool_state.outgoing_stakers.get(&proof.compute_signer())
                            }
                        };

                        if let Some(transaction) = transaction {
                            txs_to_remove.push(transaction.hash());
                        }
                    }

                    // If it is an incoming staking transaction then we have additional checks.
                    if tx.recipient_type == AccountType::Staking {
                        // Parse transaction data.
                        let data = IncomingStakingTransactionData::parse(tx)
                            .expect("The data should have already been parsed before");

                        // If the recipient was already in the mempool we need to remove all txs for this recipient
                        let transaction = match data.clone() {
                            IncomingStakingTransactionData::CreateValidator { proof, .. } => {
                                mempool_state
                                    .creating_validators
                                    .get(&proof.compute_signer())
                            }
                            IncomingStakingTransactionData::CreateStaker { proof, .. } => {
                                mempool_state.creating_stakers.get(&proof.compute_signer())
                            }
                            _ => None,
                        };

                        if let Some(transaction) = transaction {
                            txs_to_remove.push(transaction.hash());
                        }
                    }

                    for tx in txs_to_remove {
                        mempool_state.remove(&tx);
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
                    let pending_balance = tx.total_value() + sender_total;

                    if pending_balance <= sender_balance {
                        mempool_state.put(tx);
                    } else {
                        log::debug!(
                            "Tx {} from reverted block #{}.{} was dropped because of insufficient funds",
                            tx_hash,
                            block.block_number(),
                            block.view_number()
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
            let tx_hash = match mempool_state_upgraded.best_transactions.peek() {
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
        let network_id = Arc::new(blockchain.read().network_id);
        let verify_tx_ret =
            verify_tx(&transaction, blockchain, network_id, &mempool_state, filter).await;

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

    /// Returns the number of pending transactions in mempool.
    pub fn num_transactions(&self) -> usize {
        self.state.read().transactions.len()
    }

    /// Gets all transactions in the mempool.
    pub fn get_transactions(&self) -> Vec<Transaction> {
        self.state.read().transactions.values().cloned().collect()
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

pub(crate) struct MempoolState {
    // A hashmap containing the transactions indexed by their hash.
    pub(crate) transactions: HashMap<Blake2bHash, Transaction>,

    // Transactions ordered by fee per byte (highest to lowest) and insertion order (oldest to newest).
    // This is the ordering in which transactions are included in blocks by the validator.
    pub(crate) best_transactions: KeyedPriorityQueue<Blake2bHash, BestTxOrder>,

    // Transactions ordered by fee per byte (lowest to highest) and insertion order (newest to oldest).
    // This is the ordering used to evict transactions from the mempool when it becomes full.
    pub(crate) worst_transactions: KeyedPriorityQueue<Blake2bHash, WorstTxOrder>,

    // Transactions ordered by validity_start_height (oldest to newest).
    // This ordering is used to evict expired transactions from the mempool.
    pub(crate) oldest_transactions: KeyedPriorityQueue<Blake2bHash, u32>,

    // The pending balance per sender.
    pub(crate) state_by_sender: HashMap<Address, SenderPendingState>,

    // The sets of all senders of staking transactions. For simplicity, each validator/staker can
    // only have one outgoing staking transaction in the mempool. This makes sure that the outgoing
    // staking transaction can actually pay its fee.
    pub(crate) outgoing_validators: HashMap<Address, Transaction>,
    pub(crate) outgoing_stakers: HashMap<Address, Transaction>,

    // The sets of all recipients of creation staking transactions. For simplicity, each
    // validator/staker can only have one creation staking transaction in the mempool. This makes
    // sure that the creation staking transactions do not interfere with one another.
    pub(crate) creating_validators: HashMap<Address, Transaction>,
    pub(crate) creating_stakers: HashMap<Address, Transaction>,

    // Maximum allowed total size (in bytes) of all transactions in the mempool.
    pub(crate) total_size_limit: usize,

    // Total size (in bytes) of the transactions currently in mempool.
    pub(crate) total_size: usize,

    // Counter that increases for every added transaction, to order them for removal.
    pub(crate) tx_counter: u64,
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

        self.best_transactions.push(
            tx_hash.clone(),
            BestTxOrder {
                fee_per_byte: tx.fee_per_byte(),
                insertion_order: self.tx_counter,
            },
        );
        self.worst_transactions.push(
            tx_hash.clone(),
            WorstTxOrder {
                fee_per_byte: tx.fee_per_byte(),
                insertion_order: self.tx_counter,
            },
        );
        self.tx_counter += 1;

        self.oldest_transactions
            .push(tx_hash.clone(), tx.validity_start_height);

        match self.state_by_sender.get_mut(&tx.sender) {
            None => {
                let mut txns = HashSet::new();
                txns.insert(tx_hash);

                self.state_by_sender.insert(
                    tx.sender.clone(),
                    SenderPendingState {
                        total: tx.total_value(),
                        txns,
                    },
                );
            }
            Some(sender_state) => {
                sender_state.total += tx.total_value();
                sender_state.txns.insert(tx_hash);
            }
        }

        // If it is an outgoing staking transaction then we have additional work.
        if tx.sender_type == AccountType::Staking {
            // Parse transaction data.
            let data = OutgoingStakingTransactionProof::parse(tx)
                .expect("The proof should have already been parsed before, so this cannot panic!");

            // Insert the sender address in the correct set.
            match data {
                OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                    assert_eq!(
                        self.outgoing_validators
                            .insert(proof.compute_signer(), tx.clone()),
                        None
                    );
                }
                OutgoingStakingTransactionProof::Unstake { proof } => {
                    assert_eq!(
                        self.outgoing_stakers
                            .insert(proof.compute_signer(), tx.clone()),
                        None
                    );
                }
            }
        }

        // If it is an incoming staking transaction then we have additional work.
        if tx.recipient_type == AccountType::Staking {
            // Parse transaction data.
            let data = IncomingStakingTransactionData::parse(tx)
                .expect("The data should have already been parsed before, so this cannot panic!");

            // Insert the recipient address in the correct set, if it is a creation transaction.
            match data {
                IncomingStakingTransactionData::CreateValidator { proof, .. } => {
                    assert_eq!(
                        self.creating_validators
                            .insert(proof.compute_signer(), tx.clone()),
                        None
                    );
                }
                IncomingStakingTransactionData::CreateStaker { proof, .. } => {
                    assert_eq!(
                        self.creating_stakers
                            .insert(proof.compute_signer(), tx.clone()),
                        None
                    );
                }
                _ => {}
            }
        }

        // Update total tx size and remove the cheapest ones if we have too many txs.
        self.total_size += tx.serialized_size();
        while self.total_size > self.total_size_limit {
            let (tx_hash, _) = self.worst_transactions.pop().unwrap();
            self.remove(&tx_hash);
        }

        true
    }

    pub(crate) fn remove(&mut self, tx_hash: &Blake2bHash) -> Option<Transaction> {
        let tx = self.transactions.remove(tx_hash)?;

        self.best_transactions.remove(tx_hash);
        self.worst_transactions.remove(tx_hash);
        self.oldest_transactions.remove(tx_hash);

        let sender_state = self.state_by_sender.get_mut(&tx.sender).unwrap();

        sender_state.total -= tx.total_value();
        sender_state.txns.remove(tx_hash);

        if sender_state.txns.is_empty() {
            self.state_by_sender.remove(&tx.sender);
        }

        self.remove_from_staking_state(&tx);

        self.total_size -= tx.serialized_size();

        Some(tx)
    }

    // Removes all the transactions sent by some specific address
    pub(crate) fn remove_sender_txns(&mut self, sender_address: &Address) {
        if let Some(sender_state) = &self.state_by_sender.remove(sender_address) {
            for tx_hash in &sender_state.txns {
                self.best_transactions.remove(tx_hash);
                self.worst_transactions.remove(tx_hash);
                self.oldest_transactions.remove(tx_hash);
                if let Some(tx) = self.transactions.remove(tx_hash) {
                    self.remove_from_staking_state(&tx);
                }
            }
        }
    }

    // Internal helper function that takes care of removing staking transactions from the mempool state
    fn remove_from_staking_state(&mut self, tx: &Transaction) {
        // If it is an outgoing staking transaction then we have additional work.
        if tx.sender_type == AccountType::Staking {
            // Parse transaction data.
            let data = OutgoingStakingTransactionProof::parse(tx)
                .expect("The proof should have already been parsed before, so this cannot panic!");

            // Remove the sender address from the correct set.
            match data {
                OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                    assert_ne!(
                        self.outgoing_validators.remove(&proof.compute_signer()),
                        None
                    );
                }
                OutgoingStakingTransactionProof::Unstake { proof } => {
                    assert_ne!(self.outgoing_stakers.remove(&proof.compute_signer()), None);
                }
            }
        }

        // If it is an incoming staking transaction then we have additional work.
        if tx.recipient_type == AccountType::Staking {
            // Parse transaction data.
            let data = IncomingStakingTransactionData::parse(tx)
                .expect("The data should have already been parsed before, so this cannot panic!");

            // Remove the recipient address from the correct set, if it is a creation transaction.
            match data {
                IncomingStakingTransactionData::CreateValidator { proof, .. } => {
                    assert_ne!(
                        self.creating_validators.remove(&proof.compute_signer()),
                        None
                    );
                }
                IncomingStakingTransactionData::CreateStaker { proof, .. } => {
                    assert_ne!(self.creating_stakers.remove(&proof.compute_signer()), None);
                }
                _ => {}
            }
        }
    }
}

pub(crate) struct SenderPendingState {
    // The sum of the txns that are currently stored in the mempool for this sender
    pub(crate) total: Coin,

    // Transaction hashes for this sender.
    pub(crate) txns: HashSet<Blake2bHash>,
}

/// Ordering in which transactions removed from the mempool to be included in blocks.
/// This is stored on a max-heap, so the greater transaction comes first.
/// Compares by fee per byte (higher first), then by insertion order (lower i.e. older first).
// TODO: Maybe use this wrapper to do more fine ordering. For example, we might prefer small size
//       transactions over large size transactions (assuming they have the same fee per byte). Or
//       we might prefer basic transactions over staking contract transactions, etc, etc.
#[derive(PartialEq)]
pub struct BestTxOrder {
    fee_per_byte: f64,
    insertion_order: u64,
}

impl Eq for BestTxOrder {}

impl PartialOrd for BestTxOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BestTxOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        self.fee_per_byte
            .partial_cmp(&other.fee_per_byte)
            .expect("fees can't be NaN")
            .then(self.insertion_order.cmp(&other.insertion_order).reverse())
    }
}

/// Ordering in which transactions are evicted when the mempool is full.
/// This is stored on a max-heap, so the greater transaction comes first.
/// Compares by fee per byte (lower first), then by insertion order (higher i.e. newer first).
#[derive(PartialEq)]
pub struct WorstTxOrder {
    fee_per_byte: f64,
    insertion_order: u64,
}

impl Eq for WorstTxOrder {}

impl PartialOrd for WorstTxOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for WorstTxOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        self.fee_per_byte
            .partial_cmp(&other.fee_per_byte)
            .expect("fees can't be NaN")
            .reverse()
            .then(self.insertion_order.cmp(&other.insertion_order))
    }
}
