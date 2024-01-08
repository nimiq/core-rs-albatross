use std::{
    cmp::{Ordering, Reverse},
    collections::HashMap,
};

use keyed_priority_queue::KeyedPriorityQueue;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_serde::Serialize;
use nimiq_transaction::Transaction;

/// TxPriority that is used when adding transactions into the mempool
/// Higher Priority transactions are returned first from the mempool
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum TxPriority {
    /// Low Priority transactions
    Low = 1,
    /// Medium Priority transactions, this is the default
    Medium = 2,
    /// High Priority transactions,
    High = 3,
}

/// Ordering in which transactions removed from the mempool to be included in blocks.
/// This is stored on a max-heap, so the greater transaction comes first.
/// Compares by fee per byte (higher first), then by insertion order (lower i.e. older first).
// TODO: Maybe use this wrapper to do more fine ordering. For example, we might prefer small size
//       transactions over large size transactions (assuming they have the same fee per byte). Or
//       we might prefer basic transactions over staking contract transactions, etc, etc.
#[derive(PartialEq)]
pub struct BestTxOrder {
    priority: TxPriority,
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
        (self.priority as u8)
            .partial_cmp(&(other.priority as u8))
            .expect("TX Priority is required")
            .then(
                self.fee_per_byte
                    .partial_cmp(&other.fee_per_byte)
                    .expect("fees can't be NaN"),
            )
            .then(self.insertion_order.cmp(&other.insertion_order).reverse())
    }
}

/// Ordering in which transactions are evicted when the mempool is full.
/// This is stored on a max-heap, so the greater transaction comes first.
/// Compares by fee per byte (lower first), then by insertion order (higher i.e. newer first).
#[derive(PartialEq)]
pub struct WorstTxOrder {
    priority: TxPriority,
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
        (self.priority as u8)
            .partial_cmp(&(other.priority as u8))
            .expect("TX Priority is required")
            .reverse()
            .then(
                self.fee_per_byte
                    .partial_cmp(&other.fee_per_byte)
                    .expect("fees can't be NaN")
                    .reverse()
                    .then(self.insertion_order.cmp(&other.insertion_order)),
            )
    }
}

// This is a container where all mempool transactions are stored.
// It provides simple functions to insert/delete/get transactions
// And maintains internal structures to keep track of the best/worst transactions
pub(crate) struct MempoolTransactions {
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
    pub(crate) oldest_transactions: KeyedPriorityQueue<Blake2bHash, Reverse<u32>>,

    // Maximum allowed total size (in bytes) of all transactions in the mempool.
    pub(crate) total_size_limit: usize,

    // Total size (in bytes) of the transactions currently in mempool.
    pub(crate) total_size: usize,

    // Counter that increases for every added transaction, to order them for removal.
    pub(crate) tx_counter: u64,
}

impl MempoolTransactions {
    pub fn new(size_limit: usize) -> Self {
        Self {
            transactions: HashMap::new(),
            best_transactions: KeyedPriorityQueue::new(),
            worst_transactions: KeyedPriorityQueue::new(),
            oldest_transactions: KeyedPriorityQueue::new(),
            total_size_limit: size_limit,
            total_size: 0,
            tx_counter: 0,
        }
    }

    pub fn contains_key(&self, hash: &Blake2bHash) -> bool {
        self.transactions.contains_key(hash)
    }

    pub fn len(&self) -> usize {
        self.transactions.len()
    }

    // This function is used to remove the transactions that are no longer valid at a given block number.
    pub fn get_expired_txns(&mut self, block_number: u32) -> Vec<Blake2bHash> {
        let mut expired_txns = vec![];
        loop {
            // Get the hash of the oldest transaction.
            let tx_hash = match self.oldest_transactions.peek() {
                None => break,
                Some((tx_hash, _)) => tx_hash.clone(),
            };

            // Get a reference to the transaction.
            let tx = self.get(&tx_hash).unwrap();

            // Check if it is still valid.
            if tx.is_valid_at(block_number) {
                // No need to process more transactions, since we arrived to the oldest one that is valid
                break;
            } else {
                // Remove the transaction from the oldest struct to continue collecting expired txns.
                self.oldest_transactions.remove(&tx_hash);
                expired_txns.push(tx_hash);
            }
        }
        expired_txns
    }

    pub fn get(&self, hash: &Blake2bHash) -> Option<&Transaction> {
        self.transactions.get(hash)
    }

    pub(crate) fn insert(&mut self, tx: &Transaction, priority: TxPriority) -> bool {
        let tx_hash = tx.hash();

        if self.transactions.contains_key(&tx_hash) {
            return false;
        }

        self.transactions.insert(tx_hash.clone(), tx.clone());

        self.best_transactions.push(
            tx_hash.clone(),
            BestTxOrder {
                priority,
                fee_per_byte: tx.fee_per_byte(),
                insertion_order: self.tx_counter,
            },
        );
        self.worst_transactions.push(
            tx_hash.clone(),
            WorstTxOrder {
                priority,
                fee_per_byte: tx.fee_per_byte(),
                insertion_order: self.tx_counter,
            },
        );

        self.tx_counter += 1;

        self.oldest_transactions
            .push(tx_hash, Reverse(tx.validity_start_height));

        // Update total tx size
        self.total_size += tx.serialized_size();

        true
    }

    pub(crate) fn delete(&mut self, tx_hash: &Blake2bHash) -> Option<Transaction> {
        let tx = self.transactions.remove(tx_hash)?;

        self.best_transactions.remove(tx_hash);
        self.worst_transactions.remove(tx_hash);
        self.oldest_transactions.remove(tx_hash);

        self.total_size -= tx.serialized_size();

        Some(tx)
    }
}
