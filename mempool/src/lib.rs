#[macro_use]
extern crate log;
extern crate nimiq_account as account;
extern crate nimiq_block_base as block_base;
extern crate nimiq_blockchain_base as blockchain_base;
extern crate nimiq_collections as collections;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};

use account::{Account, AccountTransactionInteraction};
use beserial::Serialize;
use block_base::Block;
use blockchain_base::{AbstractBlockchain, BlockchainEvent};
use hash::{Blake2bHash, Hash};
use keys::Address;
use transaction::{Transaction, TransactionFlags};
use utils::observer::{Notifier, weak_listener};
use primitives::networks::NetworkId;

use crate::filter::{MempoolFilter, Rules};

pub mod filter;

pub struct Mempool<B: AbstractBlockchain> {
    blockchain: Arc<B>,
    pub notifier: RwLock<Notifier<'static, MempoolEvent>>,
    state: RwLock<MempoolState>,
    mut_lock: Mutex<()>,
}

struct MempoolState {
    transactions_by_hash: HashMap<Blake2bHash, Arc<Transaction>>,
    transactions_by_sender: HashMap<Address, BTreeSet<Arc<Transaction>>>,
    transactions_by_recipient: HashMap<Address, BTreeSet<Arc<Transaction>>>,
    transactions_sorted_fee: BTreeSet<Arc<Transaction>>, // sorted by fee, ascending
    filter: MempoolFilter,
}

#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
pub enum MempoolEvent {
    TransactionAdded(Blake2bHash, Arc<Transaction>),
    TransactionRestored(Arc<Transaction>),
    TransactionMined(Arc<Transaction>),
    TransactionEvicted(Arc<Transaction>),
}

#[derive(Debug, Clone)]
pub struct MempoolConfig {
    pub filter_rules: Rules,
    pub filter_limit: usize,
}

impl Default for MempoolConfig {
    fn default() -> MempoolConfig {
        MempoolConfig {
            filter_rules: Rules::default(),
            filter_limit: MempoolFilter::DEFAULT_BLACKLIST_SIZE
        }
    }
}

impl<B: AbstractBlockchain + 'static> Mempool<B> {
    pub fn new(blockchain: Arc<B>, config: MempoolConfig) -> Arc<Self> {
        let arc = Arc::new(Self {
            blockchain: blockchain.clone(),
            notifier: RwLock::new(Notifier::new()),
            state: RwLock::new(MempoolState {
                transactions_by_hash: HashMap::new(),
                transactions_by_sender: HashMap::new(),
                transactions_by_recipient: HashMap::new(),
                transactions_sorted_fee: BTreeSet::new(),
                filter: MempoolFilter::new(config.filter_rules, config.filter_limit),
            }),
            mut_lock: Mutex::new(()),
        });

        // register listener to blockchain through weak reference
        let weak = Arc::downgrade(&arc);
        blockchain.register_listener(weak_listener(weak, |this: Arc<Self>, event: &BlockchainEvent<B::Block>| {
            this.on_blockchain_event(event)
        }));

        arc
    }

    pub fn is_filtered(&self, hash: &Blake2bHash) -> bool {
        self.state.read().filter.blacklisted(hash)
    }

    pub fn push_transaction(&self, mut transaction: Transaction) -> ReturnCode {
        let hash: Blake2bHash = transaction.hash();

        // Synchronize with `Blockchain::push`
        let _push_lock = self.blockchain.lock();

        // Only one mutating operation at a time.
        let _lock = self.mut_lock.lock();

        // Transactions that are invalidated by the new transaction are stored here.
        let mut txs_to_remove = Vec::new();

        {
            let state = self.state.upgradable_read();

            // Check transaction against rules and blacklist
            if !state.filter.accepts_transaction(&transaction) || state.filter.blacklisted(&hash) {
                let mut state = RwLockUpgradableReadGuard::upgrade(state);
                state.filter.blacklist(hash);
                trace!("Transaction was filtered: {}", transaction.hash::<Blake2bHash>());
                return ReturnCode::Filtered;
            }

            // Check if we already know this transaction.
            if state.transactions_by_hash.contains_key(&hash) {
                return ReturnCode::Known;
            };

            // Intrinsic transaction verification.
            if transaction.verify_mut(self.blockchain.network_id()).is_err() {
                return ReturnCode::Invalid;
            }

            // Check limit for free transactions.
            let txs_by_sender_opt = state.transactions_by_sender.get(&transaction.sender);
            if transaction.fee_per_byte() < TRANSACTION_RELAY_FEE_MIN {
                let mut num_free_tx = 0;
                if let Some(transactions) = txs_by_sender_opt {
                    for tx in transactions {
                        if tx.fee_per_byte() < TRANSACTION_RELAY_FEE_MIN {
                            num_free_tx += 1;
                            if num_free_tx >= FREE_TRANSACTIONS_PER_SENDER_MAX {
                                return ReturnCode::FeeTooLow;
                            }
                        } else {
                            // We found the first non-free transaction in the set without hitting the limit.
                            break;
                        }
                    }
                }
            }

            // Check if transaction is valid at the next block height.
            let block_height = self.blockchain.head_height() + 1;
            if !transaction.is_valid_at(block_height) {
                return ReturnCode::Invalid;
            }

            // Check if transaction has already been mined.
            if self.blockchain.contains_tx_in_validity_window(&hash) {
                return ReturnCode::Invalid;
            }

            // Retrieve recipient account and check account type.
            // TODO Eliminate copy
            let mut recipient_account = self.blockchain.get_account(&transaction.recipient);
            let is_contract_creation = transaction.flags.contains(TransactionFlags::CONTRACT_CREATION);
            let is_type_change = recipient_account.account_type() != transaction.recipient_type;
            if is_contract_creation != is_type_change {
                return ReturnCode::Invalid;
            }

            // Test incoming transaction.
            let old_balance = recipient_account.balance();
            match recipient_account.commit_incoming_transaction(&transaction, block_height) {
                Err(_) => return ReturnCode::Invalid,
                Ok(_) => {
                    // Check recipient account against filter rules.
                    let new_balance = recipient_account.balance();
                    if !state.filter.accepts_recipient_balance(&transaction, old_balance, new_balance) {
                        self.state.write().filter.blacklist(hash);
                        return ReturnCode::Filtered;
                    }
                }
            }
            // Also check contract creation.
            if is_contract_creation {
                if Account::new_contract(transaction.recipient_type, recipient_account.balance(), &transaction, block_height).is_err() {
                    return ReturnCode::Invalid;
                }
            }

            // Retrieve sender account and check account type.
            // TODO Eliminate copy
            let mut sender_account = self.blockchain.get_account(&transaction.sender);
            if sender_account.account_type() != transaction.sender_type {
                return ReturnCode::Invalid;
            }

            // Re-check all transactions for this sender in fee/byte order against the sender account state.
            // Adding high fee transactions may thus invalidate low fee transactions in the set.
            let empty_btree; // XXX Only needed to get an empty BTree iterator
            let mut tx_count = 0;
            let mut tx_iter = match txs_by_sender_opt {
                Some(transactions) => transactions.iter(),
                None => {
                    empty_btree = BTreeSet::new();
                    empty_btree.iter()
                }
            };

            // First apply all transactions with a higher fee/byte.
            // These are not affected by the new transaction and should never fail to apply.
            let mut tx_opt = tx_iter.next_back();
            while let Some(tx) = tx_opt {
                // Break on the first transaction with a lower fee/byte.
                if transaction.cmp(tx) == Ordering::Greater {
                    break;
                }
                // Reject the transaction, if after the intrinsic check, the balance went too low
                if sender_account.commit_outgoing_transaction(tx, block_height).is_err() {
                    return ReturnCode::Invalid
                }
                tx_count += 1;
                tx_opt = tx_iter.next_back();
            }

            // If we are already at the transaction limit, reject the new transaction.
            if tx_count >= TRANSACTIONS_PER_SENDER_MAX {
                return ReturnCode::FeeTooLow;
            }

            // Now, check the new transaction.
            let old_sender_balance = sender_account.balance();
            if sender_account.commit_outgoing_transaction(&transaction, block_height).is_err() {
                return ReturnCode::Invalid // XXX More specific return code here?
            };

            // Check sender account against filter rules.
            if !state.filter.accepts_sender_balance(&transaction, old_sender_balance, sender_account.balance()) {
                self.state.write().filter.blacklist(hash);
                return ReturnCode::Filtered;
            }

            tx_count += 1;

            // Finally, check the remaining transactions with lower fee/byte and evict them if necessary.
            // tx_opt already contains the first lower/fee byte transaction to check (if there is one remaining).
            while let Some(tx) = tx_opt {
                if tx_count < TRANSACTIONS_PER_SENDER_MAX {
                    if sender_account.commit_outgoing_transaction(tx, block_height).is_ok() {
                        tx_count += 1;
                    } else {
                        txs_to_remove.push(tx.clone())
                    }
                } else {
                    txs_to_remove.push(tx.clone())
                }
                tx_opt = tx_iter.next_back();
            }
        }

        let tx_arc = Arc::new(transaction);

        let mut removed_transactions;
        {
            // Transaction is valid, add it to the mempool.
            let mut state = self.state.write();
            Self::add_transaction(&mut state, hash.clone(), tx_arc.clone());

            // Evict transactions that were invalidated by the new transaction.
            for tx in txs_to_remove.iter() {
                Self::remove_transaction(&mut *state, tx);
            }

            // Rename variable.
            removed_transactions = txs_to_remove;

            // Remove the lowest fee transaction if mempool max size is reached.
            if state.transactions_sorted_fee.len() > SIZE_MAX {
                let tx = state.transactions_sorted_fee.iter().next().unwrap().clone();
                Self::remove_transaction(&mut state, &tx);
                removed_transactions.push(tx);
            }
        }

        // Drop the lock on blockchain::push
        drop(_push_lock);

        // Tell listeners about the new transaction we received.
        self.notifier.read().notify(MempoolEvent::TransactionAdded(hash, tx_arc));

        // Tell listeners about the transactions we evicted.
        for tx in removed_transactions {
            self.notifier.read().notify(MempoolEvent::TransactionEvicted(tx));
        }

        ReturnCode::Accepted
    }

    pub fn contains(&self, hash: &Blake2bHash) -> bool {
        self.state.read().transactions_by_hash.contains_key(hash)
    }

    pub fn get_transaction(&self, hash: &Blake2bHash) -> Option<Arc<Transaction>> {
        self.state.read().transactions_by_hash.get(hash).cloned()
    }

    pub fn get_transactions(&self, max_count: usize, min_fee_per_byte: f64) -> Vec<Arc<Transaction>> {
        self.state.read().transactions_sorted_fee.iter()
            .filter(|tx| tx.fee_per_byte() >= min_fee_per_byte)
            .take(max_count)
            .cloned()
            .collect()
    }

    pub fn get_transactions_for_block(&self, max_size: usize) -> Vec<Transaction> {
        let mut txs = Vec::new();
        let mut size = 0;

        let mut validator_registry = None;
        let block_height = self.blockchain.head_height() + 1;

        let state = self.state.read();
        for tx in state.transactions_sorted_fee.iter() {
            // Check validity of transactions concerning the staking contract.
            // We only do it for the staking contract, since this is the only place
            // where the transaction validity depends on the recipient's state.
            if let Some(validator_registry_address) = self.blockchain.validator_registry_address() {
                // First apply the sender side to the staking contract if necessary.
                // This could for example drop a validator and make subsequent update transactions invalid.
                let mut outgoing_receipt = None;
                if &tx.sender == validator_registry_address {
                    // Get copy of staking contract if required.
                    if validator_registry.is_none() {
                        validator_registry = Some(self.blockchain.get_account(validator_registry_address));
                    }

                    let sender_account = validator_registry.as_mut().unwrap();
                    match sender_account.commit_outgoing_transaction(&tx, block_height) {
                        Err(_) => continue, // Ignore transaction.
                        Ok(receipt) => outgoing_receipt = receipt,
                    }
                }

                // Then apply the recipient side to the staking contract.
                if &tx.recipient == validator_registry_address {
                    // Get copy of staking contract if required.
                    if validator_registry.is_none() {
                        validator_registry = Some(self.blockchain.get_account(validator_registry_address));
                    }

                    let recipient_account = validator_registry.as_mut().unwrap();
                    if recipient_account.commit_incoming_transaction(&tx, block_height).is_err() {
                        // Potentially revert sender side and ignore transaction.
                        if &tx.sender == validator_registry_address {
                            recipient_account.revert_outgoing_transaction(&tx, block_height, outgoing_receipt.as_ref()).unwrap();
                        }
                        continue;
                    }
                }
            }

            let tx_size = tx.serialized_size();
            if size + tx_size <= max_size {
                txs.push(Transaction::clone(tx));
                size += tx_size;
            } else if max_size - size < Transaction::MIN_SIZE {
                // Break if we can't fit the smallest possible transaction anymore.
                break;
            }
        };
        txs
    }

    pub fn get_transactions_by_addresses(&self, addresses: HashSet<Address>, max_count: usize) -> Vec<Arc<Transaction>> {
        let mut txs = Vec::new();

        let state = self.state.read();
        for address in addresses {
            // Fetch transactions by sender first.
            if let Some(transactions) = state.transactions_by_sender.get(&address) {
                for tx in transactions.iter().rev().take(max_count - txs.len()) {
                    txs.push(Arc::clone(tx));
                }
            }
            // Fetch transactions by recipient second.
            if let Some(transactions) = state.transactions_by_recipient.get(&address) {
                for tx in transactions.iter().rev().take(max_count - txs.len()) {
                    txs.push(Arc::clone(tx));
                }
            }
            if txs.len() >= max_count {
                break
            };
        }
        txs
    }

    pub fn current_height(&self) -> u32 {
        self.blockchain.head_height()
    }

    pub fn network_id(&self) -> NetworkId {
        self.blockchain.network_id()
    }

    fn on_blockchain_event(&self, event: &BlockchainEvent<B::Block>) {
        match event {
            BlockchainEvent::Extended(_) | BlockchainEvent::Finalized(_) => {
                self.evict_transactions()
            },
            BlockchainEvent::Rebranched(reverted_blocks, _) => {
                self.restore_transactions(reverted_blocks);
                self.evict_transactions();
            },
        }
    }

    /// Evict all transactions from the pool that have become invalid due to changes in the
    /// account state (i.e. typically because the were included in a newly mined block). No need to re-check signatures.
    fn evict_transactions(&self) {
        // Only one mutating operation at a time.
        let _lock = self.mut_lock.lock();

        let mut txs_mined = Vec::new();
        let mut txs_evicted = Vec::new();
        {
            let state = self.state.read();
            let block_height = self.blockchain.head_height() + 1;

            for (address, transactions) in state.transactions_by_sender.iter() {
                // TODO Eliminate copy
                let mut sender_account = self.blockchain.get_account(&address);
                for tx in transactions.iter().rev() {
                    // Check if the transaction has expired.
                    if !tx.is_valid_at(block_height) {
                        txs_evicted.push(tx.clone());
                        continue;
                    }

                    // Check if the transaction has been mined.
                    if self.blockchain.contains_tx_in_validity_window(&tx.hash()) {
                        txs_mined.push(tx.clone());
                        continue;
                    }

                    // Check if transaction is still valid for recipient.
                    // TODO Eliminate copy
                    let mut recipient_account = self.blockchain.get_account(&tx.recipient);
                    if recipient_account.commit_incoming_transaction(&tx, block_height).is_err() {
                        txs_evicted.push(tx.clone());
                        continue;
                    }
                    // Also check contract creation.
                    if tx.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                        if Account::new_contract(tx.recipient_type, recipient_account.balance(), &tx, block_height).is_err() {
                            txs_evicted.push(tx.clone());
                            continue;
                        }
                    }

                    // Check if transaction is still valid for sender.
                    if sender_account.commit_outgoing_transaction(&tx, block_height).is_err() {
                        txs_evicted.push(tx.clone());
                    }
                }
            }
        }

        {
            // Evict transactions.
            let mut state = self.state.write();
            for tx in txs_mined.iter() {
                Self::remove_transaction(&mut state, tx);
            }
            for tx in txs_evicted.iter() {
                Self::remove_transaction(&mut state, tx);
            }
        }

        // Notify listeners.
        for tx in txs_mined {
            trace!("Transaction minded: {:?}", tx);
            self.notifier.read().notify(MempoolEvent::TransactionMined(tx));
        }

        for tx in txs_evicted {
            trace!("Transaction evicted: {:?}", tx);
            self.notifier.read().notify(MempoolEvent::TransactionEvicted(tx));
        }
    }

    fn restore_transactions(&self, reverted_blocks: &[(Blake2bHash, B::Block)]) {
        // Only one mutating operation at a time.
        let _lock = self.mut_lock.lock();

        let mut removed_transactions = Vec::new();
        let mut restored_transactions = Vec::new();
        let block_height = self.blockchain.head_height() + 1;

        // Collect all transactions from reverted blocks that are still valid.
        // Track them by sender and sort them by fee/byte.
        let mut txs_by_sender = HashMap::new();
        for (_, block) in reverted_blocks {
            let transactions = block.transactions();
            if transactions.is_none() {
                continue;
            }

            for tx in transactions.unwrap().iter() {
                if !tx.is_valid_at(block_height) {
                    // This transaction has expired (or is not valid yet) on the new chain.
                    // XXX The transaction is lost!
                    continue;
                }

                if self.blockchain.contains_tx_in_validity_window(&tx.hash()) {
                    // This transaction is also included in the new chain, ignore.
                    continue;
                }

                // TODO Eliminate copy
                let mut recipient_account = self.blockchain.get_account(&tx.recipient);
                if recipient_account.commit_incoming_transaction(&tx, block_height).is_err() {
                    // This transaction cannot be accepted by the recipient anymore.
                    // XXX The transaction is lost!
                    continue;
                }
                // Also check contract creation.
                if tx.flags.contains(TransactionFlags::CONTRACT_CREATION) {
                    if Account::new_contract(tx.recipient_type, recipient_account.balance(), &tx, block_height).is_err() {
                        // This transaction cannot be accepted by the recipient anymore.
                        // XXX The transaction is lost!
                        continue;
                    }
                }

                let txs = txs_by_sender
                    .entry(&tx.sender)
                    .or_insert_with(BTreeSet::new);
                txs.insert(tx);
            }
        }

        // Merge the new transaction sets per sender with the existing ones.
        {
            let mut state = self.state.write();

            for (sender, restored_txs) in txs_by_sender {
                let empty_btree;
                let existing_txs = match state.transactions_by_sender.get(&sender) {
                    Some(txs) => txs,
                    None => {
                        empty_btree = BTreeSet::new();
                        &empty_btree
                    }
                };

                // TODO Eliminate copy.
                let sender_account = self.blockchain.get_account(&sender);
                let (txs_to_add, txs_to_remove) = Self::merge_transactions(sender_account, block_height, existing_txs, &restored_txs);
                for tx in txs_to_add {
                    let transaction = Arc::new(tx.clone());
                    Self::add_transaction(&mut state, tx.hash(), transaction.clone());
                    restored_transactions.push(transaction);
                }
                for tx in txs_to_remove {
                    Self::remove_transaction(&mut state, &tx);
                    removed_transactions.push(tx);
                }
            }

            // Evict lowest fee transactions if the mempool has grown too large.
            let size = state.transactions_sorted_fee.len();
            if size > SIZE_MAX {
                let mut txs_to_remove = Vec::with_capacity(size - SIZE_MAX);
                let mut iter = state.transactions_sorted_fee.iter();
                for _ in 0..size - SIZE_MAX {
                    txs_to_remove.push(iter.next().unwrap().clone());
                }
                for tx in txs_to_remove.iter() {
                    Self::remove_transaction(&mut state, tx);
                }
                removed_transactions.extend(txs_to_remove);
            }
        }

        // Notify listeners.
        for tx in removed_transactions {
            self.notifier.read().notify(MempoolEvent::TransactionEvicted(tx));
        }

        for tx in restored_transactions {
            self.notifier.read().notify(MempoolEvent::TransactionRestored(tx));
        }
    }

    fn add_transaction(state: &mut MempoolState, hash: Blake2bHash, tx: Arc<Transaction>) {
        state.transactions_by_hash.insert(hash, tx.clone());
        state.transactions_sorted_fee.insert(tx.clone());

        let txs_by_recipient = state.transactions_by_recipient
            .entry(tx.recipient.clone()) // XXX Get rid of the .clone() here
            .or_insert_with(BTreeSet::new);
        txs_by_recipient.insert(tx.clone());

        let txs_by_sender = state.transactions_by_sender
            .entry(tx.sender.clone()) // XXX Get rid of the .clone() here
            .or_insert_with(BTreeSet::new);
        txs_by_sender.insert(tx.clone());
    }

    fn remove_transaction(state: &mut MempoolState, tx: &Transaction) {
        state.transactions_by_hash.remove(&tx.hash());
        state.transactions_sorted_fee.remove(tx);

        let mut remove_key = false;
        if let Some(transactions) = state.transactions_by_sender.get_mut(&tx.sender) {
            transactions.remove(tx);
            remove_key = transactions.is_empty();
        }
        if remove_key {
            state.transactions_by_sender.remove(&tx.sender);
        }

        remove_key = false;
        if let Some(transactions) = state.transactions_by_recipient.get_mut(&tx.recipient) {
            transactions.remove(tx);
            remove_key = transactions.is_empty();
        }
        if remove_key {
            state.transactions_by_recipient.remove(&tx.recipient);
        }
    }

    fn merge_transactions<'a>(mut sender_account: Account, block_height: u32, old_txs: &BTreeSet<Arc<Transaction>>, new_txs: &BTreeSet<&'a Transaction>) -> (Vec<&'a Transaction>, Vec<Arc<Transaction>>) {
        let mut txs_to_add = Vec::new();
        let mut txs_to_remove = Vec::new();

        // TODO Eliminate copy
        let mut tx_count = 0;

        let mut iter_old = old_txs.iter();
        let mut iter_new = new_txs.iter();
        let mut old_tx = iter_old.next_back();
        let mut new_tx = iter_new.next_back();

        while old_tx.is_some() || new_tx.is_some() {
            let new_is_next = match (old_tx, new_tx) {
                (Some(_), None) => false,
                (None, Some(_)) => true,
                (Some(txc), Some(txn)) => txn.cmp(&txc.as_ref()) == Ordering::Greater,
                (None, None) => unreachable!()
            };

            if new_is_next {
                if tx_count < TRANSACTIONS_PER_SENDER_MAX {
                    let tx = new_tx.unwrap();
                    if sender_account.commit_outgoing_transaction(*tx, block_height).is_ok() {
                        tx_count += 1;
                        txs_to_add.push(*tx)
                    }
                }
                new_tx = iter_new.next_back();
            } else {
                let tx = old_tx.unwrap();
                if tx_count < TRANSACTIONS_PER_SENDER_MAX {
                    if sender_account.commit_outgoing_transaction(tx, block_height).is_ok() {
                        tx_count += 1;
                    } else {
                        txs_to_remove.push(tx.clone())
                    }
                } else {
                    txs_to_remove.push(tx.clone())
                }
                old_tx = iter_old.next_back();
            }
        }

        (txs_to_add, txs_to_remove)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReturnCode {
    FeeTooLow,
    Invalid,
    Accepted,
    Known,
    Filtered,
}

/// Fee threshold in sat/byte below which transactions are considered "free".
const TRANSACTION_RELAY_FEE_MIN : f64 = 1f64;

/// Maximum number of transactions per sender.
const TRANSACTIONS_PER_SENDER_MAX : u32 = 500;

/// Maximum number of "free" transactions per sender.
const FREE_TRANSACTIONS_PER_SENDER_MAX : u32 = 10;

/// Maximum number of transactions in the mempool.
pub const SIZE_MAX : usize = 100_000;
