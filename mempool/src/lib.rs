#[macro_use]
extern crate log;
extern crate nimiq_account as account;
extern crate nimiq_accounts as accounts;
extern crate nimiq_block as block;
extern crate nimiq_blockchain as blockchain;
extern crate nimiq_collections as collections;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use account::AccountTransactionInteraction;
use accounts::Accounts;
use beserial::Serialize;
use block::Block;
use blockchain::{Blockchain, BlockchainEvent};
use hash::{Blake2bHash, Hash};
use keys::Address;
use transaction::Transaction;
use utils::observer::Notifier;

use crate::filter::{MempoolFilter, Rules};
use parking_lot::RwLockUpgradableReadGuard;

pub mod filter;

pub struct Mempool<'env> {
    blockchain: Arc<Blockchain<'env>>,
    pub notifier: RwLock<Notifier<'env, MempoolEvent>>,
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

impl<'env> Mempool<'env> {
    pub fn new(blockchain: Arc<Blockchain<'env>>, config: MempoolConfig) -> Arc<Self> {
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

        let arc_self = arc.clone();
        blockchain.notifier.write().register(move |event: &BlockchainEvent| arc_self.on_blockchain_event(event));
        arc
    }

    pub fn is_filtered(&self, hash: &Blake2bHash) -> bool {
        self.state.read().filter.blacklisted(hash)
    }

    pub fn push_transaction(&self, mut transaction: Transaction) -> ReturnCode {
        let hash: Blake2bHash = transaction.hash();

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
            if transaction.verify_mut(self.blockchain.network_id).is_err() {
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

            let recipient_account;
            let mut sender_account;
            let block_height;
            {
                // Acquire blockchain read lock.
                let blockchain_state = self.blockchain.state();
                let accounts = blockchain_state.accounts();
                let transaction_cache = blockchain_state.transaction_cache();
                block_height = blockchain_state.main_chain().head.header.height + 1;

                // Check if transaction is valid at the next block height.
                if !transaction.is_valid_at(block_height) {
                    return ReturnCode::Invalid;
                }

                // Check if transaction has already been mined.
                if transaction_cache.contains(&hash) {
                    return ReturnCode::Invalid;
                }

                // Retrieve recipient account and test incoming transaction.
                recipient_account = accounts.get(&transaction.recipient, None);
                if recipient_account.account_type() != transaction.recipient_type {
                    return ReturnCode::Invalid;
                }

                match recipient_account.with_incoming_transaction(&transaction, block_height) {
                    Err(_) => return ReturnCode::Invalid,
                    Ok(r) => {
                        // Check recipient account against filter rules.
                        if !state.filter.accepts_recipient_account(&transaction, &recipient_account, &r) {
                            self.state.write().filter.blacklist(hash);
                            return ReturnCode::Filtered;
                        }
                    }
                }

                // Retrieve sender account and test account type.
                sender_account = accounts.get(&transaction.sender, None);
                if sender_account.account_type() != transaction.sender_type {
                    return ReturnCode::Invalid;
                }
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
                sender_account = match sender_account
                    .with_outgoing_transaction(tx, block_height) {
                    Ok(s) => s,
                    Err(_) => return ReturnCode::Invalid
                };
                tx_count += 1;

                tx_opt = tx_iter.next_back();
            }

            // If we are already at the transaction limit, reject the new transaction.
            if tx_count >= TRANSACTIONS_PER_SENDER_MAX {
                return ReturnCode::FeeTooLow;
            }

            // Now, check the new transaction.
            let old_sender_account = sender_account.clone();
            sender_account = match sender_account.with_outgoing_transaction(&transaction, block_height) {
                Ok(account) => account,
                Err(_) => return ReturnCode::Invalid // XXX More specific return code here?
            };

            // Check sender account against filter rules.
            if !state.filter.accepts_sender_account(&transaction, &old_sender_account, &sender_account) {
                self.state.write().filter.blacklist(hash);
                return ReturnCode::Filtered;
            }

            tx_count += 1;

            // Finally, check the remaining transactions with lower fee/byte and evict them if necessary.
            // tx_opt already contains the first lower/fee byte transaction to check (if there is one remaining).
            while let Some(tx) = tx_opt {
                if tx_count < TRANSACTIONS_PER_SENDER_MAX {
                    if let Ok(account) = sender_account.with_outgoing_transaction(tx, block_height) {
                        sender_account = account;
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
            Mempool::add_transaction(&mut state, hash.clone(), tx_arc.clone());

            // Evict transactions that were invalidated by the new transaction.
            for tx in txs_to_remove.iter() {
                Mempool::remove_transaction(&mut *state, tx);
            }

            // Rename variable.
            removed_transactions = txs_to_remove;

            // Remove the lowest fee transaction if mempool max size is reached.
            if state.transactions_sorted_fee.len() > SIZE_MAX {
                let tx = state.transactions_sorted_fee.iter().next().unwrap().clone();
                Mempool::remove_transaction(&mut state, &tx);
                removed_transactions.push(tx);
            }
        }

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

    pub fn get_transactions(&self, max_size: usize, min_fee_per_byte: f64) -> Vec<Arc<Transaction>> {
        let mut txs = Vec::new();
        let mut size = 0;

        let state = self.state.read();
        let valid_txs: Vec<&Arc<Transaction>> = state.transactions_sorted_fee.iter().filter(|tx| tx.fee_per_byte() >= min_fee_per_byte).collect();
        for tx in valid_txs {
            let tx_size = tx.serialized_size();
            if size + tx_size <= max_size {
                txs.push(tx.clone());
                size += tx_size;
            } else if max_size - size < Transaction::MIN_SIZE {
                // Break if we can't fit the smallest possible transaction anymore.
                break;
            }
        };

        txs
    }

    pub fn get_all_transactions(&self) -> Vec<Arc<Transaction>> {
        let state = self.state.read();
        state.transactions_sorted_fee.iter().map(Arc::clone).collect()
    }

    pub fn get_transactions_for_block(&self, max_size: usize) -> Vec<Arc<Transaction>> {
        let transactions = self.get_transactions(max_size, 0f64);
        // TODO get to be pruned accounts and remove transactions to fit max_size
        unimplemented!();
    }

    pub fn get_transactions_by_addresses(&self, addresses: HashSet<Address>, max_transactions: usize) -> Vec<Arc<Transaction>> {
        let mut txs = Vec::new();

        let state = self.state.read();
        for address in addresses {
            // Fetch transactions by sender first.
            if let Some(transactions) = state.transactions_by_sender.get(&address) {
                for tx in transactions.iter().rev() {
                    txs.push(tx.clone());
                }
            }
            // Fetch transactions by recipient second.
            if let Some(transactions) = state.transactions_by_recipient.get(&address) {
                for tx in transactions.iter().rev() {
                    txs.push(tx.clone());
                }
            }
        }
        // TODO: optimize this to not push txs that are going to be discarded anyways
        txs.truncate(max_transactions);
        txs
    }

    fn on_blockchain_event(&self, event: &BlockchainEvent) {
        match event {
            BlockchainEvent::Extended(_) => self.evict_transactions(),
            BlockchainEvent::Rebranched(reverted_blocks, _) => self.restore_transactions(reverted_blocks),
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

            // Acquire blockchain read lock.
            let blockchain_state = self.blockchain.state();
            let accounts = blockchain_state.accounts();
            let transaction_cache = blockchain_state.transaction_cache();
            let block_height = blockchain_state.main_chain().head.header.height + 1;

            for (address, transactions) in state.transactions_by_sender.iter() {
                let mut sender_account = accounts.get(&address, None);
                for tx in transactions.iter().rev() {
                    // Check if the transaction has expired.
                    if !tx.is_valid_at(block_height) {
                        txs_evicted.push(tx.clone());
                        continue;
                    }

                    // Check if the transaction has been mined.
                    if transaction_cache.contains(&tx.hash()) {
                        txs_mined.push(tx.clone());
                        continue;
                    }

                    // Check if transaction is still valid for recipient.
                    let recipient_account = accounts.get(&tx.recipient, None);

                    if recipient_account.with_incoming_transaction(&tx, block_height).is_err() {
                        txs_evicted.push(tx.clone());
                        continue;
                    }

                    // Check if transaction is still valid for sender.
                    let sender_account_res = sender_account.with_outgoing_transaction(&tx, block_height);
                    if sender_account_res.is_err() {
                        txs_evicted.push(tx.clone());
                        continue;
                    }

                    // Transaction ok.
                    sender_account = sender_account_res.unwrap();
                }
            }
        }

        {
            // Evict transactions.
            let mut state = self.state.write();
            for tx in txs_mined.iter() {
                Mempool::remove_transaction(&mut state, tx);
            }
            for tx in txs_evicted.iter() {
                Mempool::remove_transaction(&mut state, tx);
            }
        }

        // Notify listeners.
        for tx in txs_mined {
            self.notifier.read().notify(MempoolEvent::TransactionMined(tx));
        }

        for tx in txs_evicted {
            self.notifier.read().notify(MempoolEvent::TransactionEvicted(tx));
        }
    }

    fn restore_transactions(&self, reverted_blocks: &[(Blake2bHash, Block)]) {
        // Only one mutating operation at a time.
        let _lock = self.mut_lock.lock();

        let mut removed_transactions = Vec::new();
        let mut restored_transactions = Vec::new();

        {
            // Acquire blockchain read lock.
            let blockchain_state = self.blockchain.state();
            let accounts = blockchain_state.accounts();
            let transaction_cache = blockchain_state.transaction_cache();
            let block_height = blockchain_state.main_chain().head.header.height + 1;

            // Collect all transactions from reverted blocks that are still valid.
            // Track them by sender and sort them by fee/byte.
            let mut txs_by_sender = HashMap::new();
            for (_, block) in reverted_blocks {
                for tx in block.body.as_ref().unwrap().transactions.iter() {
                    if !tx.is_valid_at(block_height) {
                        // This transaction has expired (or is not valid yet) on the new chain.
                        // XXX The transaction is lost!
                        continue;
                    }

                    if transaction_cache.contains(&tx.hash()) {
                        // This transaction is also included in the new chain, ignore.
                        continue;
                    }

                    let recipient_account = accounts.get(&tx.recipient, None);
                    if recipient_account.with_incoming_transaction(&tx, block_height).is_err() {
                        // This transaction cannot be accepted by the recipient anymore.
                        // XXX The transaction is lost!
                        continue;
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

                    let (txs_to_add, txs_to_remove) = Mempool::merge_transactions(&accounts, sender, block_height, existing_txs, &restored_txs);
                    for tx in txs_to_add {
                        let transaction = Arc::new(tx.clone());
                        Mempool::add_transaction(&mut state, tx.hash(), transaction.clone());
                        restored_transactions.push(transaction);
                    }
                    for tx in txs_to_remove {
                        Mempool::remove_transaction(&mut state, &tx);
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
                        Mempool::remove_transaction(&mut state, tx);
                    }
                    removed_transactions.extend(txs_to_remove);
                }
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

    fn merge_transactions<'a>(accounts: &Accounts, sender: &Address, block_height: u32, old_txs: &BTreeSet<Arc<Transaction>>, new_txs: &BTreeSet<&'a Transaction>) -> (Vec<&'a Transaction>, Vec<Arc<Transaction>>) {
        let mut txs_to_add = Vec::new();
        let mut txs_to_remove = Vec::new();

        let mut sender_account = accounts.get(sender, None);
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
                    if let Ok(account) = sender_account.with_outgoing_transaction(*tx, block_height) {
                        sender_account = account;
                        tx_count += 1;
                        txs_to_add.push(*tx)
                    }
                }
                new_tx = iter_new.next_back();
            } else {
                let tx = old_tx.unwrap();
                if tx_count < TRANSACTIONS_PER_SENDER_MAX {
                    if let Ok(account) = sender_account.with_outgoing_transaction(tx, block_height) {
                        sender_account = account;
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
