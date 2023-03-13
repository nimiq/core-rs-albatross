use nimiq_account::ReservedBalance;
use std::collections::{HashMap, HashSet};
#[cfg(feature = "metrics")]
use std::sync::Arc;

#[cfg(feature = "metrics")]
use crate::mempool_metrics::MempoolMetrics;
use crate::mempool_transactions::{MempoolTransactions, TxPriority};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_primitives::account::AccountType;
use nimiq_transaction::Transaction;

pub(crate) struct MempoolState {
    // Container where the regular transactions are stored
    pub(crate) regular_transactions: MempoolTransactions,

    // Container where the control transactions are stored
    pub(crate) control_transactions: MempoolTransactions,

    // The pending balance per sender.
    pub(crate) state_by_sender: HashMap<Address, SenderPendingState>,

    #[cfg(feature = "metrics")]
    pub(crate) metrics: Arc<MempoolMetrics>,
}

impl MempoolState {
    pub fn new(regular_txns_limit: usize, control_txns_limit: usize) -> Self {
        MempoolState {
            regular_transactions: MempoolTransactions::new(regular_txns_limit),
            control_transactions: MempoolTransactions::new(control_txns_limit),
            state_by_sender: HashMap::new(),
            #[cfg(feature = "metrics")]
            metrics: Default::default(),
        }
    }

    pub fn contains(&self, hash: &Blake2bHash) -> bool {
        self.regular_transactions.contains_key(hash) || self.control_transactions.contains_key(hash)
    }

    pub fn get(&self, hash: &Blake2bHash) -> Option<&Transaction> {
        if let Some(transaction) = self.regular_transactions.get(hash) {
            Some(transaction)
        } else if let Some(transaction) = self.control_transactions.get(hash) {
            Some(transaction)
        } else {
            None
        }
    }

    pub(crate) fn put(&mut self, tx: &Transaction, priority: TxPriority) -> bool {
        let tx_hash = tx.hash();

        if self.regular_transactions.contains_key(&tx_hash)
            || self.control_transactions.contains_key(&tx_hash)
        {
            return false;
        }

        // Update the per sender state
        match self.state_by_sender.get_mut(&tx.sender) {
            None => {
                let mut txns = HashSet::new();
                txns.insert(tx_hash);

                let mut reserved_balance = ReservedBalance::new(tx.sender.clone());
                let reserve_failed = reserved_balance
                    .reserve_unchecked(tx.total_value())
                    .is_err();
                if reserve_failed {
                    return false;
                }

                self.state_by_sender.insert(
                    tx.sender.clone(),
                    SenderPendingState {
                        reserved_balance,
                        txns,
                    },
                );
            }
            Some(sender_state) => {
                let reserve_failed = sender_state
                    .reserved_balance
                    .reserve_unchecked(tx.total_value())
                    .is_err();
                if reserve_failed {
                    return false;
                }
                sender_state.txns.insert(tx_hash);
            }
        }

        // If we are adding a staking transaction we insert it into the control txns container
        // Staking txns are control txns
        if tx.sender_type == AccountType::Staking || tx.recipient_type == AccountType::Staking {
            self.control_transactions.insert(tx, priority);
        } else {
            self.regular_transactions.insert(tx, priority);
        }

        // After inserting the new txn, check if we need to remove txns
        while self.regular_transactions.total_size > self.regular_transactions.total_size_limit {
            let (tx_hash, _) = self.regular_transactions.worst_transactions.pop().unwrap();
            self.remove(&tx_hash, EvictionReason::TooFull);
        }

        while self.control_transactions.total_size > self.control_transactions.total_size_limit {
            let (tx_hash, _) = self.control_transactions.worst_transactions.pop().unwrap();
            self.remove(&tx_hash, EvictionReason::TooFull);
        }

        true
    }

    pub(crate) fn remove(
        &mut self,
        tx_hash: &Blake2bHash,
        reason: EvictionReason,
    ) -> Option<Transaction> {
        if let Some(tx) = self
            .regular_transactions
            .delete(tx_hash)
            .or_else(|| self.control_transactions.delete(tx_hash))
        {
            let sender_state = self.state_by_sender.get_mut(&tx.sender).unwrap();

            sender_state.reserved_balance.release(tx.total_value());
            sender_state.txns.remove(tx_hash);

            if sender_state.txns.is_empty() {
                self.state_by_sender.remove(&tx.sender);
            }

            #[cfg(feature = "metrics")]
            self.metrics.note_evicted(reason);

            Some(tx)
        } else {
            None
        }
    }

    // Removes all the transactions sent by some specific address
    pub(crate) fn remove_sender_txns(&mut self, sender_address: &Address) {
        if let Some(sender_state) = &self.state_by_sender.remove(sender_address) {
            for tx_hash in &sender_state.txns {
                self.regular_transactions
                    .delete(tx_hash)
                    .or_else(|| self.control_transactions.delete(tx_hash));
            }
        }
    }

    /// Retrieves all expired transaction hashes from both the `regular_transactions` and `control_transactions` Vectors
    pub fn get_expired_txns(&mut self, block_height: u32) -> Vec<Blake2bHash> {
        let mut expired_txns = self.control_transactions.get_expired_txns(block_height);
        expired_txns.append(&mut self.regular_transactions.get_expired_txns(block_height));

        expired_txns
    }
}

#[derive(Clone)]
pub(crate) enum EvictionReason {
    BlockBuilding,
    Expired,
    AlreadyIncluded,
    Invalid,
    TooFull,
}

pub(crate) struct SenderPendingState {
    // The balance reserved by transactions that are currently stored in the mempool for this sender.
    pub(crate) reserved_balance: ReservedBalance,

    // Transaction hashes for this sender.
    pub(crate) txns: HashSet<Blake2bHash>,
}
