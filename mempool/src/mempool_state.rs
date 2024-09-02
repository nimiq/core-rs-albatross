use std::collections::{HashMap, HashSet};
#[cfg(feature = "metrics")]
use std::sync::Arc;

use nimiq_account::ReservedBalance;
use nimiq_blockchain::Blockchain;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_primitives::account::AccountType;
use nimiq_transaction::Transaction;

#[cfg(feature = "metrics")]
use crate::mempool_metrics::MempoolMetrics;
use crate::{
    mempool_transactions::{MempoolTransactions, TxPriority},
    verify::VerifyErr,
};

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

    pub(crate) fn put(
        &mut self,
        blockchain: &Blockchain,
        tx: Transaction,
        priority: TxPriority,
    ) -> Result<(), VerifyErr> {
        // Don't add the same transaction twice.
        let tx_hash = tx.hash();
        if self.contains(&tx_hash) {
            return Err(VerifyErr::Known);
        }

        // Reserve the balance necessary for this transaction on the sender account.
        let sender_account = blockchain
            .get_account_if_complete(&tx.sender)
            .ok_or(VerifyErr::NoConsensus)?;

        if let Some(sender_state) = self.state_by_sender.get_mut(&tx.sender) {
            let reserved_balance = &mut sender_state.reserved_balance;
            blockchain.reserve_balance(&sender_account, &tx, reserved_balance)?;
            sender_state.txns.insert(tx.hash());
        } else {
            let mut reserved_balance = ReservedBalance::new(tx.sender.clone());
            blockchain.reserve_balance(&sender_account, &tx, &mut reserved_balance)?;

            let sender_state = SenderPendingState {
                reserved_balance,
                txns: HashSet::from([tx.hash()]),
            };
            self.state_by_sender.insert(tx.sender.clone(), sender_state);
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
            self.remove(blockchain, &tx_hash, EvictionReason::TooFull);
        }

        while self.control_transactions.total_size > self.control_transactions.total_size_limit {
            let (tx_hash, _) = self.control_transactions.worst_transactions.pop().unwrap();
            self.remove(blockchain, &tx_hash, EvictionReason::TooFull);
        }

        Ok(())
    }

    pub(crate) fn remove(
        &mut self,
        blockchain: &Blockchain,
        tx_hash: &Blake2bHash,
        #[allow(unused_variables)] reason: EvictionReason,
    ) -> Option<Transaction> {
        let tx = self
            .regular_transactions
            .delete(tx_hash)
            .or_else(|| self.control_transactions.delete(tx_hash))?;

        let sender_state = match self.state_by_sender.get_mut(&tx.sender) {
            Some(state) => state,
            None => return Some(tx),
        };

        let sender_account = match blockchain.get_account_if_complete(&tx.sender) {
            Some(account) => account,
            None => {
                // We don't know the sender account so we can't do any balance tracking.
                // Throw away all transactions from this sender.
                warn!(
                    sender_address = %tx.sender,
                    num_transactions = sender_state.txns.len(),
                    "Sender account is gone"
                );
                for hash in &sender_state.txns {
                    self.regular_transactions
                        .delete(hash)
                        .or_else(|| self.control_transactions.delete(hash));
                }
                self.state_by_sender.remove(&tx.sender);
                return Some(tx);
            }
        };

        if !sender_state.txns.remove(tx_hash) {
            return Some(tx);
        }

        blockchain
            .release_balance(&sender_account, &tx, &mut sender_state.reserved_balance)
            .expect("Failed to release balance");

        if sender_state.txns.is_empty() {
            self.state_by_sender.remove(&tx.sender);
        }

        #[cfg(feature = "metrics")]
        self.metrics.note_evicted(reason);

        Some(tx)
    }

    /// Retrieves all expired transaction hashes from both the `regular_transactions` and `control_transactions` vectors
    pub fn get_expired_txns(&mut self, block_number: u32) -> Vec<Blake2bHash> {
        let mut expired_txns = self.control_transactions.get_expired_txns(block_number);
        expired_txns.append(&mut self.regular_transactions.get_expired_txns(block_number));
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
