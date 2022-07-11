use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_primitives::{account::AccountType, coin::Coin};
use nimiq_transaction::{
    account::staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionProof},
    Transaction,
};

use crate::{mempool_metrics::MempoolMetrics, mempool_transactions::MempoolTransactions};

pub(crate) struct MempoolState {
    // Container where the regular transactions are stored
    pub(crate) regular_transactions: MempoolTransactions,

    // Container where the control transactions are stored
    pub(crate) control_transactions: MempoolTransactions,

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

    #[cfg(feature = "metrics")]
    pub(crate) metrics: Arc<MempoolMetrics>,
}

impl MempoolState {
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

    pub(crate) fn put(&mut self, tx: &Transaction) -> bool {
        let tx_hash = tx.hash();

        if self.regular_transactions.contains_key(&tx_hash)
            || self.control_transactions.contains_key(&tx_hash)
        {
            return false;
        }

        // If we are adding a stacking transaction we insert it into the control txns container
        // Staking txns are control txns
        if tx.sender_type == AccountType::Staking || tx.recipient_type == AccountType::Staking {
            self.control_transactions.insert(tx);
        } else {
            self.regular_transactions.insert(tx);
        }

        // Update the per sender state
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

            sender_state.total -= tx.total_value();
            sender_state.txns.remove(tx_hash);

            if sender_state.txns.is_empty() {
                self.state_by_sender.remove(&tx.sender);
            }

            self.remove_from_staking_state(&tx);

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
                if let Some(tx) = self
                    .regular_transactions
                    .delete(tx_hash)
                    .or_else(|| self.control_transactions.delete(tx_hash))
                {
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
    // The sum of the txns that are currently stored in the mempool for this sender
    pub(crate) total: Coin,

    // Transaction hashes for this sender.
    pub(crate) txns: HashSet<Blake2bHash>,
}
