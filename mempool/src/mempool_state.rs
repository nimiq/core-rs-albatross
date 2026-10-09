use std::collections::{HashMap, HashSet};
#[cfg(feature = "metrics")]
use std::sync::Arc;

use nimiq_account::ReservedBalance;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_primitives::account::AccountType;
use nimiq_transaction::{account::staking_contract::IncomingStakingTransactionData, Transaction};

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

    // Maximum number of transactions stored per sender.
    pub(crate) tx_per_sender_limit: usize,

    // The protocol version under which all stored transactions have been verified. The
    // verification cache only vouches for a block transaction whose version matches this.
    pub(crate) verified_version: u16,

    // The pending voting-key rotation per validator. Its proof of knowledge is the most expensive
    // thing the mempool verifies, so a validator gets one in flight at a time.
    pub(crate) pending_voting_key_updates: HashMap<Address, Blake2bHash>,

    // The keys whose transaction carrying a proof of knowledge failed behind valid signatures, by
    // block number. Only the key holder can sign such a failure, so the key gets one per block.
    pub(crate) failed_proofs_of_knowledge: HashMap<Address, u32>,

    #[cfg(feature = "metrics")]
    pub(crate) metrics: Arc<MempoolMetrics>,
}

impl MempoolState {
    pub fn new(
        regular_txns_limit: usize,
        control_txns_limit: usize,
        tx_per_sender_limit: usize,
        verified_version: u16,
    ) -> Self {
        MempoolState {
            regular_transactions: MempoolTransactions::new(regular_txns_limit),
            control_transactions: MempoolTransactions::new(control_txns_limit),
            state_by_sender: HashMap::new(),
            tx_per_sender_limit,
            verified_version,
            pending_voting_key_updates: HashMap::new(),
            failed_proofs_of_knowledge: HashMap::new(),
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

    /// The checks of `put` without the insertion, so that they can run before the costly
    /// signature verification. `put` repeats them under the write lock.
    pub(crate) fn check_put(
        &self,
        blockchain: &Blockchain,
        tx: &Transaction,
    ) -> Result<(), VerifyErr> {
        if tx.uses_small_order_key() {
            return Err(VerifyErr::SmallOrderKey);
        }

        if let Some(sender_state) = self.state_by_sender.get(&tx.sender)
            && sender_state.txns.len() >= self.tx_per_sender_limit
        {
            return Err(VerifyErr::SenderLimitReached);
        }

        if let Some(validator) = voting_key_update_target(tx)
            && self.has_pending_voting_key_update(&validator)
        {
            return Err(VerifyErr::VotingKeyUpdatePending);
        }

        if let Some(key) = proof_of_knowledge_key(tx)
            && self.failed_proofs_of_knowledge.get(&key) == Some(&blockchain.block_number())
        {
            return Err(VerifyErr::ProofOfKnowledgeLimitReached);
        }

        let sender_account = blockchain
            .get_account_if_complete(&tx.sender)
            .ok_or(VerifyErr::NoConsensus)?;

        // Dry run against the sender's pending reservation, not a fresh one: a funded sender
        // gets one admitted transaction per balance held, and every further one fails here.
        let mut reserved_balance = match self.state_by_sender.get(&tx.sender) {
            Some(sender_state) => sender_state.reserved_balance.clone(),
            None => ReservedBalance::new(tx.sender.clone()),
        };
        blockchain.reserve_balance(&sender_account, tx, &mut reserved_balance)?;
        Ok(())
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

        // Refuse transactions relying on small-order Ed25519
        // keys. Checked here, not in `verify_tx`, because reverted blocks re-add through `put()`.
        if tx.uses_small_order_key() {
            debug!(%tx_hash, "Refusing transaction that uses a small-order Ed25519 key");
            return Err(VerifyErr::SmallOrderKey);
        }

        // Reject if this sender already has too many transactions in the mempool.
        if let Some(sender_state) = self.state_by_sender.get(&tx.sender)
            && sender_state.txns.len() >= self.tx_per_sender_limit
        {
            return Err(VerifyErr::SenderLimitReached);
        }

        // Reject a second voting-key rotation for a validator while one is pending.
        let voting_key_update = voting_key_update_target(&tx);
        if let Some(validator) = &voting_key_update
            && self.has_pending_voting_key_update(validator)
        {
            return Err(VerifyErr::VotingKeyUpdatePending);
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

        if let Some(validator) = voting_key_update {
            self.pending_voting_key_updates.insert(validator, tx_hash);
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

        // The transaction has left the mempool, whichever way this function returns below.
        #[cfg(feature = "metrics")]
        self.metrics.note_evicted(reason);

        if let Some(validator) = voting_key_update_target(&tx)
            && self.pending_voting_key_updates.get(&validator) == Some(tx_hash)
        {
            self.pending_voting_key_updates.remove(&validator);
        }

        let sender_state = match self.state_by_sender.get_mut(&tx.sender) {
            Some(state) => state,
            None => return Some(tx),
        };

        let sender_account = match blockchain.get_account_if_complete(&tx.sender) {
            Some(account) => account,
            None => {
                // We don't know the sender account so we can't do any balance tracking.
                // Throw away all transactions from this sender.
                for hash in &sender_state.txns {
                    if self
                        .regular_transactions
                        .delete(hash)
                        .or_else(|| self.control_transactions.delete(hash))
                        .is_some()
                    {
                        #[cfg(feature = "metrics")]
                        self.metrics
                            .note_evicted(EvictionReason::SenderAccountUnknown);
                    }
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

        Some(tx)
    }

    /// A sender whose account is unknown has its transactions dropped without `remove` (see
    /// there and `Mempool::recompute_sender_balances`), so an entry only counts while its
    /// transaction is still in the pool.
    fn has_pending_voting_key_update(&self, validator: &Address) -> bool {
        self.pending_voting_key_updates
            .get(validator)
            .is_some_and(|hash| self.contains(hash))
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
    /// Dropped together with another transaction of the same sender, because the sender's
    /// account is not known (the accounts trie is incomplete), so its balance can't be tracked.
    /// Only used to count such removals.
    #[cfg_attr(not(feature = "metrics"), allow(dead_code))]
    SenderAccountUnknown,
}

pub(crate) struct SenderPendingState {
    // The balance reserved by transactions that are currently stored in the mempool for this sender.
    pub(crate) reserved_balance: ReservedBalance,

    // Transaction hashes for this sender.
    pub(crate) txns: HashSet<Blake2bHash>,
}

/// The validator whose voting key `tx` rotates, if it is such an `UpdateValidator`.
fn voting_key_update_target(tx: &Transaction) -> Option<Address> {
    if tx.recipient_type != AccountType::Staking {
        return None;
    }
    match IncomingStakingTransactionData::parse(tx) {
        Ok(IncomingStakingTransactionData::UpdateValidator {
            new_voting_key: Some(_),
            proof,
            ..
        }) => Some(proof.compute_signer()),
        _ => None,
    }
}

/// The account a proof of knowledge is charged to: the sender of a `CreateValidator`, or the
/// validator whose voting key an `UpdateValidator` rotates.
pub(crate) fn proof_of_knowledge_key(tx: &Transaction) -> Option<Address> {
    if tx.recipient_type != AccountType::Staking {
        return None;
    }
    match IncomingStakingTransactionData::parse(tx) {
        Ok(IncomingStakingTransactionData::CreateValidator { .. }) => Some(tx.sender.clone()),
        Ok(IncomingStakingTransactionData::UpdateValidator {
            new_voting_key: Some(_),
            proof,
            ..
        }) => Some(proof.compute_signer()),
        _ => None,
    }
}

#[cfg(all(test, feature = "metrics"))]
mod tests {
    use std::collections::HashSet;

    use nimiq_account::ReservedBalance;
    use nimiq_blockchain::Blockchain;
    use nimiq_hash::{Blake2bHash, Hash};
    use nimiq_keys::Address;
    use nimiq_primitives::{coin::Coin, networks::NetworkId};
    use nimiq_test_log::test;
    use nimiq_test_utils::block_production::TemporaryBlockProducer;
    use nimiq_transaction::Transaction;
    use prometheus_client::{encoding::text::encode, registry::Registry};

    use super::{EvictionReason, MempoolState, SenderPendingState};
    use crate::mempool_transactions::TxPriority;

    fn sender() -> Address {
        Address::from([1u8; Address::SIZE])
    }

    fn transaction(value: u64) -> Transaction {
        Transaction::new_basic(
            sender(),
            Address::burn_address(),
            Coin::from_u64_unchecked(value),
            Coin::ZERO,
            1,
            NetworkId::UnitAlbatross,
        )
    }

    fn new_state() -> MempoolState {
        MempoolState::new(1_000_000, 1_000_000, 100, 1)
    }

    /// Puts `txs` into the mempool's container, bypassing the balance checks of `put`.
    fn insert(state: &mut MempoolState, txs: &[Transaction]) -> Vec<Blake2bHash> {
        txs.iter()
            .map(|tx| {
                state
                    .regular_transactions
                    .insert(tx.clone(), TxPriority::Medium);
                tx.hash()
            })
            .collect()
    }

    fn add_sender_state(state: &mut MempoolState, txns: HashSet<Blake2bHash>) {
        let sender_state = SenderPendingState {
            reserved_balance: ReservedBalance::new(sender()),
            txns,
        };
        state.state_by_sender.insert(sender(), sender_state);
    }

    /// Returns how many transactions were counted as removed for `reason`.
    fn removed(state: &MempoolState, reason: &str) -> u64 {
        let mut registry = Registry::default();
        state.metrics.register(&mut registry);
        let mut encoded = String::new();
        encode(&mut encoded, &registry).unwrap();
        let prefix = format!("removed_tx_count_total{{reason=\"{reason}\"}} ");
        encoded
            .lines()
            .find_map(|line| line.strip_prefix(&prefix))
            .map_or(0, |count| count.parse().unwrap())
    }

    fn remove(state: &mut MempoolState, blockchain: &Blockchain, hash: &Blake2bHash) {
        let removed = state.remove(blockchain, hash, EvictionReason::Expired);
        assert!(removed.is_some());
    }

    #[test]
    fn counts_removal_without_sender_state() {
        let producer = TemporaryBlockProducer::new();
        let mut state = new_state();
        let hashes = insert(&mut state, &[transaction(1)]);

        remove(&mut state, &producer.blockchain.read(), &hashes[0]);

        assert_eq!(removed(&state, "Expired"), 1);
    }

    #[test]
    fn counts_removal_not_tracked_for_its_sender() {
        let producer = TemporaryBlockProducer::new();
        let mut state = new_state();
        let hashes = insert(&mut state, &[transaction(1)]);
        add_sender_state(&mut state, HashSet::new());

        remove(&mut state, &producer.blockchain.read(), &hashes[0]);

        assert_eq!(removed(&state, "Expired"), 1);
    }

    #[test]
    fn counts_transactions_dropped_with_an_unknown_sender_account() {
        let producer = TemporaryBlockProducer::new_incomplete();
        let mut state = new_state();
        let hashes = insert(&mut state, &[transaction(1), transaction(2)]);
        add_sender_state(&mut state, hashes.iter().cloned().collect());

        remove(&mut state, &producer.blockchain.read(), &hashes[0]);

        assert_eq!(removed(&state, "Expired"), 1);
        assert_eq!(removed(&state, "SenderAccountUnknown"), 1);
        assert!(!state.contains(&hashes[1]));
    }
}
