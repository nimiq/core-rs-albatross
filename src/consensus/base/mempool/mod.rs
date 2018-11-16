use beserial::Serialize;
use parking_lot::RwLock;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::convert::From;
use std::sync::Arc;
use crate::consensus::base::blockchain::{Blockchain, BlockchainEvent};
use crate::consensus::base::primitive::hash::{Blake2bHash, Hash};
use crate::consensus::base::primitive::Address;
use crate::consensus::base::transaction::Transaction;

pub struct Mempool<'env> {
    blockchain: Arc<RwLock<Blockchain<'env>>>,
    transactions_by_hash: HashMap<Blake2bHash, Arc<Transaction>>,
    transactions_by_sender: HashMap<Address, BTreeSet<Arc<TransactionSortable>>>,
    transactions_by_recipient: HashMap<Address, BTreeSet<Arc<TransactionSortable>>>,
    transactions_sorted_fee: BTreeSet<Arc<TransactionSortable>>
}

impl<'env> Mempool<'env> {
    pub fn new(blockchain: Arc<RwLock<Blockchain<'env>>>) -> Arc<RwLock<Self>> {
        let arc = Arc::new(RwLock::new(Self {
            blockchain: blockchain.clone(),
            transactions_by_hash: HashMap::new(),
            transactions_by_sender: HashMap::new(),
            transactions_by_recipient: HashMap::new(),
            transactions_sorted_fee: BTreeSet::new()
        }));

        let arc_listener = arc.clone();
        blockchain.write().notifier.register(move |event: BlockchainEvent| arc_listener.write().on_blockchain_event(event));
        arc
    }

    fn on_blockchain_event(&mut self, event: BlockchainEvent) {
        println!("Mempool received blockchain event: {:?}", event);
    }

    pub fn push_transaction(&mut self, transaction: Transaction) -> ReturnCode {
        // Check if we already know this transaction.
        let hash: Blake2bHash = transaction.hash();
        if self.transactions_by_hash.contains_key(&hash) {
            return ReturnCode::Known;
        };

        // Check limit for free transactions.
        if u64::from(transaction.fee) / (transaction.serialized_size() as u64) < TRANSACTION_RELAY_FEE_MIN {
            let mut no_tx_below_fee_per_byte = 0;
            if let Some(transactions_sorted) = self.transactions_by_sender.get(&transaction.sender) {
                for transaction in transactions_sorted {
                    no_tx_below_fee_per_byte += 1;
                    if no_tx_below_fee_per_byte >= FREE_TRANSACTIONS_PER_SENDER_MAX {
                        return ReturnCode::FeeTooLow;
                    }
                }
            }
        }

        // Acquire read lock on the blockchain and hold it until the end of the function.
        // This ensures that:
        // - all transaction validation checks run against the same blockchain state
        // - the mempool is in a consistent state when a HeadChanged/Rebranched event causes it to evict transactions
        let blockchain_arc = self.blockchain.clone();
        let blockchain = blockchain_arc.read();
        let block_height = blockchain.height() + 1;

        // Intrinsic transaction verification.
        if transaction.verify(blockchain.network_id).is_err() {
            return ReturnCode::Invalid;
        }

        // Retrieve recipient account and test incoming transaction.
        let recipient_account = blockchain.accounts.get(&transaction.recipient, None);
        if recipient_account.account_type() != transaction.recipient_type {
            return ReturnCode::Invalid;
        }
        if let Err(_) = recipient_account.with_incoming_transaction(&transaction, block_height) {
            return ReturnCode::Invalid;
        }

        // Retrieve sender account and test outgoing transaction.
        let sender_account = blockchain.accounts.get(&transaction.sender, None);
        if sender_account.account_type() != transaction.sender_type {
            return ReturnCode::Invalid;
        }
        if let Err(e) = sender_account.with_outgoing_transaction(&transaction, block_height) {
            return ReturnCode::Invalid;
        }

        let transaction_arc = Arc::new(transaction);
        let transaction_sortable = Arc::new(TransactionSortable(Arc::clone(&transaction_arc)));

        // Add new transaction to the sender's pending transaction set. Then re-check all transactions in the set
        // in fee/byte order against the sender account state. Adding high fee transactions may thus invalidate
        // low fee transactions in the set.
        let mut remove_txs = Vec::new();
        let mut sender_changed = sender_account;
        if let Some(s) = self.transactions_by_sender.get(&transaction_arc.sender) {
            let mut transactions_sorted = s.clone();
            transactions_sorted.insert(Arc::clone(&transaction_sortable));

            let mut tx_count = 0;
            for curr_tx in transactions_sorted {
                if tx_count < TRANSACTIONS_PER_SENDER_MAX {
                    if let Ok(new_account) = sender_changed.with_outgoing_transaction(&transaction_arc, block_height) {
                        sender_changed = new_account;
                        tx_count += 1;
                        continue;
                    }
                }
                if tx_count >= TRANSACTIONS_PER_SENDER_MAX {
                    if curr_tx.0 == transaction_arc {
                        return ReturnCode::Invalid;
                    } else {
                        remove_txs.push(Arc::clone(&curr_tx));
                    }
                }
            }
        }
        for transaction_arc in remove_txs {
            self.remove_transaction(&transaction_arc);
        }

        // Transaction is valid, add it to the mempool.
        self.transactions_by_hash.insert(hash, Arc::clone(&transaction_arc));
        self.transactions_sorted_fee.insert(Arc::clone(&transaction_sortable));

        if let None = self.transactions_by_recipient.get(&transaction_arc.recipient) {
            self.transactions_by_recipient.insert(transaction_arc.recipient.clone(), BTreeSet::new());
        };
        if let Entry::Occupied(mut e) = self.transactions_by_recipient.entry(transaction_arc.recipient.clone()) {
            e.get_mut().insert(Arc::clone(&transaction_sortable));
        };
        if let None = self.transactions_by_sender.get(&transaction_arc.sender) {
            self.transactions_by_sender.insert(transaction_arc.sender.clone(), BTreeSet::new());
        };
        if let Entry::Occupied(mut e) = self.transactions_by_sender.entry(transaction_arc.sender.clone()) {
            e.get_mut().insert(Arc::clone(&transaction_sortable));
        };

        // Tell listeners about the new valid transaction we received.
        // TODO

        if self.transactions_sorted_fee.len() > SIZE_MAX {
            self.pop_low_fee_transaction();
        }

        return ReturnCode::Accepted;
    }

    pub fn get_transaction(&self, hash: &Blake2bHash) -> Option<Arc<Transaction>> {
        match self.transactions_by_hash.get(hash) {
            Some(t) => Some(Arc::clone(t)),
            None => None
        }
    }

    pub fn get_transactions(&self, max_size: u32, min_fee_per_byte: u32) -> Vec<Arc<Transaction>> {
        let mut ret = Vec::new();
        let size_sum = 0;
        for transaction in &self.transactions_sorted_fee {
            if size_sum + transaction.0.serialized_size() <= max_size as usize {
                ret.push(Arc::clone(&transaction.0));
            }
        };
        return ret;
    }

    pub fn get_transactions_for_block(&self, max_size: u32) -> Vec<Arc<Transaction>> {
        let transactions = self.get_transactions(max_size, 0);
        // TODO get to be pruned accounts and remove transactions to fit max_size
        unimplemented!();
    }

    pub fn get_transactions_by_addresses(&self, addresses: Vec<Address>, max_transactions: u32) -> Vec<Arc<Transaction>> {
        let mut ret = Vec::new();
        for address in addresses {
            // Fetch transactions by sender first
            if let Some(txs_arc) = self.transactions_by_sender.get(&address) {
                for ts_arc in txs_arc {
                    ret.push(Arc::clone(&ts_arc.0));
                }
            }
            // Fetch transactions by recipient second
            if let Some(txs_arc) = self.transactions_by_recipient.get(&address) {
                for ts_arc in txs_arc {
                    ret.push(Arc::clone(&ts_arc.0));
                }
            }
        }
        return ret;
    }

    /// Evict all transactions from the pool that have become invalid due to changes in the
    /// account state (i.e. typically because the were included in a newly mined block). No need to re-check signatures.
    fn evict_transactions(&mut self) {
        let blockchain_arc = self.blockchain.clone();
        let blockchain = blockchain_arc.read();

        for (address, transactions_sorted) in self.transactions_by_sender.clone().iter() {
            let mut sender_account = blockchain.accounts.get(&address, None);

            let mut transactions_sorted_new = BTreeSet::new();
            for curr_ts in transactions_sorted {
                let new_sender_account_option = sender_account.with_outgoing_transaction(&curr_ts.0, 1);

                let recipient_account = blockchain.accounts.get(&curr_ts.0.recipient, None);
                let new_recipient_account_option = recipient_account.with_incoming_transaction(&curr_ts.0, 1);

                if new_sender_account_option.is_ok() || new_recipient_account_option.is_ok() {
                    transactions_sorted_new.insert(Arc::clone(&curr_ts));
                    sender_account = new_sender_account_option.unwrap();
                } else {
                    self.remove_transaction(&curr_ts);
                }
            }
            if transactions_sorted_new.len() > 0 {
                self.transactions_by_sender.insert(address.clone(), transactions_sorted_new);
            } else {
                self.transactions_by_sender.remove(&address);
            }
        }
    }

    fn pop_low_fee_transaction(&mut self) {
        let mut owned = Option::None;
        if let Some(t) = self.transactions_sorted_fee.iter().next() {
            owned = Some(t.clone());
        }
        if let Some(t) = owned {
            self.remove_transaction(&t);
        }
    }

    fn remove_transaction(&mut self, ts: &TransactionSortable) {
        self.transactions_by_hash.remove(&ts.0.hash());
        self.transactions_sorted_fee.remove(ts);

        let mut remove_key = false;
        if let Entry::Occupied(mut e) = self.transactions_by_sender.entry(ts.0.sender.clone()) {
            let transactions_sorted = e.get_mut();
            if transactions_sorted.len() > 1 {
                transactions_sorted.remove(ts);
            } else {
                remove_key = true;
            }
        }
        if remove_key {
            self.transactions_by_sender.remove(&ts.0.sender);
        }
        if let Entry::Occupied(mut e) = self.transactions_by_recipient.entry(ts.0.recipient.clone()) {
            let transactions_sorted = e.get_mut();
            if transactions_sorted.len() > 1 {
                transactions_sorted.remove(ts);
            } else {
                remove_key = true;
            }
        }
        if remove_key {
            self.transactions_by_recipient.remove(&ts.0.sender);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReturnCode {
    FeeTooLow,
    Invalid,
    Accepted,
    Known
}

#[derive(PartialEq, Eq, PartialOrd, Debug)]
struct TransactionSortable(Arc<Transaction>);

impl Ord for TransactionSortable {
    fn cmp(&self, other: &Self) -> Ordering {
        return Ordering::Equal
            .then_with(|| (u64::from(self.0.fee) / self.0.serialized_size() as u64).cmp(&(u64::from(other.0.fee) / other.0.serialized_size() as u64)))
            .then_with(|| self.0.fee.cmp(&other.0.fee))
            .then_with(|| self.0.value.cmp(&other.0.value));
    }
}

/// Fee threshold in sat/byte below which transactions are considered "free".
const TRANSACTION_RELAY_FEE_MIN : u64 = 1;

/// Maximum number of transactions per sender.
const TRANSACTIONS_PER_SENDER_MAX : u32 = 500;

/// Maximum number of "free" transactions per sender.
const FREE_TRANSACTIONS_PER_SENDER_MAX : u32 = 10;

/// Maximum number of transactions in the mempool.
const SIZE_MAX : usize = 100000;
