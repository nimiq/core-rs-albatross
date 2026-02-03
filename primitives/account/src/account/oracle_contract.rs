use nimiq_keys::Address;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountType;
use nimiq_primitives::{account::AccountError, coin::Coin, transaction::TransactionError};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::account::htlc_contract::AnyHash;
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::account::oracle_contract::{
    CreationTransactionData, IncomingOracleTransactionData,
};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::{inherent::Inherent, HashType, SignatureProof, Transaction};

use crate::{convert_receipt, AccountReceipt};
#[cfg(feature = "interaction-traits")]
use crate::{
    data_store::{DataStoreRead, DataStoreWrite},
    interaction_traits::{
        AccountInherentInteraction, AccountPruningInteraction, AccountTransactionInteraction,
    },
    reserved_balance::ReservedBalance,
    Account, BlockState, InherentLogger, Log, TransactionLog,
};

/// The Oracle contract.
/// This contract is essentially a hash storage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OracleContract {
    /// The owner of the contract, the only address that can interact with it.
    pub owner: Address,

    /// When the contract is created, a deposit is required.
    /// If the balance goes to zero (effectively if the deposit is removed) the contract is deleted.
    /// We only support removing the deposit in full
    pub balance: Coin,

    /// The number of hashes that can be stored. This is set at creation and determines the required deposit.
    pub hash_count: u16,

    /// Ring buffer storage for hashes.
    /// Before the first write, this is empty. After that, it has exactly `hash_count` elements.
    /// Access entry at global index `i` via `hashes[i % hash_count]`.
    pub hashes: Vec<AnyHash>,

    /// The latest valid index in the hash chain.
    /// `None` means no hashes have been written yet.
    /// `Some(n)` means the latest valid index is `n`.
    pub latest_index: Option<u64>,
}

#[cfg(feature = "interaction-traits")]
impl OracleContract {
    fn can_change_balance(
        &self,
        transaction: &Transaction,
        new_balance: Coin,
        is_reserve: bool,
    ) -> Result<(), AccountError> {
        // Check transaction signer is contract owner.
        let signature_proof = SignatureProof::deserialize_all(&transaction.proof)?;

        if !signature_proof.is_signed_by(&self.owner) {
            return Err(AccountError::InvalidSignature);
        }

        // If withdrawing, must withdraw the full balance (contract deletion)
        if new_balance < self.balance {
            // For reserve_balance, we allow reserving any amount up to the full balance
            // The actual withdrawal check happens in commit_outgoing_transaction
            if is_reserve {
                return Ok(());
            }
            // For actual withdrawal, only allow full withdrawal (balance goes to zero)
            if new_balance != Coin::ZERO {
                return Err(AccountError::InvalidForSender);
            }
            // Only allow full withdrawal (balance goes to zero)
            // Check that the transaction value equals the current balance
            if transaction.value != self.balance {
                return Err(AccountError::InvalidForSender);
            }
        }

        Ok(())
    }

    /// Gets the hash type from any hash in the contract, if any.
    /// Returns None if the contract has no hashes yet.
    fn get_hash_type(&self) -> Option<HashType> {
        self.hashes.first().map(HashType::from_any_hash)
    }

    /// Validates that all hashes in the provided vector are of the same type,
    /// and that they match the contract's hash type (if the contract already has hashes).
    fn validate_hash_types(&self, new_hashes: &[AnyHash]) -> Result<(), AccountError> {
        if new_hashes.is_empty() {
            return Ok(());
        }

        // Determine the expected hash type
        let expected_type = if let Some(contract_type) = self.get_hash_type() {
            // Contract already has hashes, use that type
            contract_type
        } else {
            // Contract is empty, use the type of the first new hash
            HashType::from_any_hash(&new_hashes[0])
        };

        // Validate all new hashes match the expected type
        if new_hashes
            .iter()
            .all(|hash| HashType::from_any_hash(hash) == expected_type)
        {
            Ok(())
        } else {
            Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ))
        }
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountTransactionInteraction for OracleContract {
    fn create_new_contract(
        transaction: &Transaction,
        initial_balance: Coin,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Account, AccountError> {
        let data = CreationTransactionData::parse(transaction)
            .map_err(AccountError::InvalidTransaction)?;

        // Verify the creation data
        data.verify().map_err(AccountError::InvalidTransaction)?;

        // The deposit is the transaction value
        let deposit = transaction.value;

        tx_logger.push_log(Log::OracleCreate {
            contract_address: transaction.recipient.clone(),
            owner: data.owner.clone(),
            hash_count: data.hash_count,
            deposit,
        });

        Ok(Account::Oracle(OracleContract {
            balance: initial_balance + deposit,
            owner: data.owner,
            hash_count: data.hash_count,
            hashes: Vec::with_capacity(data.hash_count as usize),
            latest_index: None,
        }))
    }

    fn revert_new_contract(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance -= transaction.value;

        tx_logger.push_log(Log::OracleCreate {
            contract_address: transaction.recipient.clone(),
            owner: self.owner.clone(),
            hash_count: self.hash_count,
            deposit: transaction.value,
        });

        Ok(())
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        // Check if this is a signaling transaction for updates or owner changes
        if transaction
            .flags
            .contains(nimiq_transaction::TransactionFlags::SIGNALING)
        {
            let data = IncomingOracleTransactionData::parse(transaction)
                .map_err(AccountError::InvalidTransaction)?;

            match data {
                IncomingOracleTransactionData::Update { hashes, proof } => {
                    // Verify signature
                    if !proof.is_signed_by(&self.owner) {
                        return Err(AccountError::InvalidSignature);
                    }

                    // Validate that all new hashes match the contract's hash type
                    self.validate_hash_types(&hashes)?;

                    // Empty update: no state change, no log
                    if hashes.is_empty() {
                        return Ok(None);
                    }

                    // Zero hash for chaining (same type as first new hash or existing)
                    let zero_hash = hashes
                        .first()
                        .map(|h| h.zero_of_same_type())
                        .or_else(|| self.hashes.first().map(|h| h.zero_of_same_type()))
                        .unwrap_or_default();

                    // First write: establish fixed-size ring buffer.
                    if self.latest_index.is_none() {
                        self.hashes
                            .resize(self.hash_count as usize, zero_hash.clone());
                    }

                    let mut current_hash = if let Some(prev_index) = self.latest_index {
                        let prev_pos = (prev_index % self.hash_count as u64) as usize;
                        self.hashes[prev_pos].clone()
                    } else {
                        zero_hash
                    };

                    let start_index = self.latest_index.map(|i| i + 1).unwrap_or(0);
                    let mut removed_hashes = Vec::new();
                    let mut first_removed_pos = None;

                    for (offset, new_hash) in hashes.iter().enumerate() {
                        let index = start_index + offset as u64;
                        let pos = (index % self.hash_count as u64) as usize;

                        if index >= self.hash_count as u64 {
                            first_removed_pos.get_or_insert(pos as u16);
                            removed_hashes.push(self.hashes[pos].clone());
                        }

                        // data_i = H(data_{i-1} || state_i)
                        current_hash = current_hash.digest(new_hash);
                        self.hashes[pos] = current_hash.clone();
                    }

                    if !hashes.is_empty() {
                        self.latest_index = Some(start_index + hashes.len() as u64 - 1);
                    }

                    tx_logger.push_log(Log::OracleUpdate {
                        contract_address: transaction.recipient.clone(),
                        hashes,
                    });

                    // Return receipt with removed hashes for proper revert (ring buffer)
                    if removed_hashes.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(
                            UpdateReceipt {
                                removed_hashes,
                                first_removed_pos: first_removed_pos
                                    .expect("evictions must record their first position"),
                            }
                            .into(),
                        ))
                    }
                }
                IncomingOracleTransactionData::ChangeOwner { new_owner, proof } => {
                    // Verify signature
                    if !proof.is_signed_by(&self.owner) {
                        return Err(AccountError::InvalidSignature);
                    }

                    let old_owner = self.owner.clone();
                    self.owner = new_owner.clone();

                    tx_logger.push_log(Log::OracleChangeOwner {
                        contract_address: transaction.recipient.clone(),
                        old_owner: old_owner.clone(),
                        new_owner: new_owner.clone(),
                    });

                    // Return receipt with old owner for proper revert
                    Ok(Some(ChangeOwnerReceipt { old_owner }.into()))
                }
            }
        } else {
            // Regular incoming transactions are not allowed
            Err(AccountError::InvalidForRecipient)
        }
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Revert signaling transactions
        if transaction
            .flags
            .contains(nimiq_transaction::TransactionFlags::SIGNALING)
        {
            let data = IncomingOracleTransactionData::parse(transaction)
                .map_err(AccountError::InvalidTransaction)?;

            match data {
                IncomingOracleTransactionData::Update { hashes, .. } => {
                    let num_hashes = hashes.len() as u64;

                    if let Some(current_latest) = self.latest_index {
                        if num_hashes == 0 {
                            // Empty update is a no-op.
                            return Ok(());
                        }
                        if num_hashes > current_latest {
                            self.latest_index = None;
                            self.hashes.clear();
                            tx_logger.push_log(Log::OracleUpdate {
                                contract_address: transaction.recipient.clone(),
                                hashes,
                            });
                            return Ok(());
                        }

                        let start_index = current_latest - (num_hashes - 1);
                        let num_evicted = if start_index >= self.hash_count as u64 {
                            num_hashes
                        } else {
                            start_index
                                .saturating_add(num_hashes)
                                .saturating_sub(self.hash_count as u64)
                        };

                        if num_evicted > 0 {
                            let update_receipt = UpdateReceipt::try_from(
                                receipt.ok_or(AccountError::InvalidReceipt)?,
                            )?;

                            if update_receipt.removed_hashes.len() != num_evicted as usize {
                                return Err(AccountError::InvalidReceipt);
                            }

                            for (i, old_hash) in update_receipt.removed_hashes.iter().enumerate() {
                                let pos = ((u64::from(update_receipt.first_removed_pos) + i as u64)
                                    % self.hash_count as u64)
                                    as usize;
                                self.hashes[pos] = old_hash.clone();
                            }
                        }

                        // Clear positions written by the reverted update without eviction (they overwrote the resize default)
                        let num_without_eviction = (self.hash_count as u64)
                            .saturating_sub(start_index)
                            .min(num_hashes);
                        if num_without_eviction > 0 {
                            let zero_hash = self
                                .hashes
                                .first()
                                .map(|h| h.zero_of_same_type())
                                .unwrap_or_default();
                            for offset in 0..num_without_eviction {
                                let pos = (start_index + offset) % self.hash_count as u64;
                                self.hashes[pos as usize] = zero_hash.clone();
                            }
                        }

                        let new_latest = current_latest - num_hashes;
                        self.latest_index = Some(new_latest);
                    }

                    tx_logger.push_log(Log::OracleUpdate {
                        contract_address: transaction.recipient.clone(),
                        hashes,
                    });
                }
                IncomingOracleTransactionData::ChangeOwner { new_owner, .. } => {
                    // Revert owner change using receipt
                    if let Some(receipt) = receipt {
                        let change_owner_receipt = ChangeOwnerReceipt::try_from(receipt)?;
                        self.owner = change_owner_receipt.old_owner.clone();

                        tx_logger.push_log(Log::OracleChangeOwner {
                            contract_address: transaction.recipient.clone(),
                            old_owner: change_owner_receipt.old_owner.clone(),
                            new_owner: new_owner.clone(),
                        });
                    } else {
                        return Err(AccountError::InvalidReceipt);
                    }
                }
            }
        }

        Ok(())
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let new_balance = self.balance.safe_sub(transaction.total_value())?;
        self.can_change_balance(transaction, new_balance, false)?;
        self.balance = new_balance;

        tx_logger.push_log(Log::pay_fee_log(transaction));
        tx_logger.push_log(Log::transfer_log(transaction));

        Ok(None)
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance += transaction.total_value();

        tx_logger.push_log(Log::transfer_log(transaction));
        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(())
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let new_balance = self.balance.safe_sub(transaction.fee)?;

        self.can_change_balance(transaction, new_balance, false)?;
        self.balance = new_balance;

        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(None)
    }

    fn revert_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.balance += transaction.fee;

        tx_logger.push_log(Log::pay_fee_log(transaction));

        Ok(())
    }

    fn reserve_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _block_state: &BlockState,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        let needed = reserved_balance
            .balance()
            .checked_add(transaction.total_value())
            .ok_or(AccountError::InvalidCoinValue)?;
        let new_balance = self.balance.safe_sub(needed)?;
        self.can_change_balance(transaction, new_balance, true)?;

        reserved_balance.reserve(self.balance, transaction.total_value())
    }

    fn release_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        reserved_balance.release(transaction.total_value());
        Ok(())
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountInherentInteraction for OracleContract {
    fn commit_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _inherent_logger: &mut InherentLogger,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        Err(AccountError::InvalidForTarget)
    }

    fn revert_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_state: &BlockState,
        _receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
        _inherent_logger: &mut InherentLogger,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForTarget)
    }
}

#[cfg(feature = "interaction-traits")]
impl AccountPruningInteraction for OracleContract {
    fn can_be_pruned(&self) -> bool {
        self.balance.is_zero()
    }

    fn prune(self, _data_store: DataStoreRead) -> Option<AccountReceipt> {
        Some(PrunedOracleContract::from(self).into())
    }

    fn restore(
        _ty: AccountType,
        pruned_account: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        let receipt = pruned_account.ok_or(AccountError::InvalidReceipt)?;
        let pruned_account = PrunedOracleContract::try_from(receipt)?;
        Ok(Account::Oracle(OracleContract::from(pruned_account)))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct PrunedOracleContract {
    pub owner: Address,
    pub hash_count: u16,
    pub hashes: Vec<AnyHash>,
    pub latest_index: Option<u64>,
}

impl From<OracleContract> for PrunedOracleContract {
    fn from(contract: OracleContract) -> Self {
        PrunedOracleContract {
            owner: contract.owner,
            hash_count: contract.hash_count,
            hashes: contract.hashes,
            latest_index: contract.latest_index,
        }
    }
}

impl From<PrunedOracleContract> for OracleContract {
    fn from(receipt: PrunedOracleContract) -> Self {
        OracleContract {
            balance: Coin::ZERO,
            owner: receipt.owner,
            hash_count: receipt.hash_count,
            hashes: receipt.hashes,
            latest_index: receipt.latest_index,
        }
    }
}

convert_receipt!(PrunedOracleContract);

/// Receipt for update transactions. This is necessary to be able to revert
/// these transactions when hashes were removed due to ring buffer behavior.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct UpdateReceipt {
    /// The hashes that were removed from the beginning when the ring buffer limit was reached
    pub removed_hashes: Vec<AnyHash>,

    /// The ring-buffer position of `removed_hashes[0]`.
    pub first_removed_pos: u16,
}

convert_receipt!(UpdateReceipt);

/// Receipt for owner change transactions. This is necessary to be able to revert
/// these transactions.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
struct ChangeOwnerReceipt {
    /// The owner before this transaction is applied
    pub old_owner: Address,
}

convert_receipt!(ChangeOwnerReceipt);

// Helper methods for accessing ring buffer data
impl OracleContract {
    /// Returns the earliest index still retained in the ring buffer.
    /// All indices < earliest_index have been overwritten.
    pub fn earliest_index(&self) -> Option<u64> {
        if let Some(latest) = self.latest_index {
            if latest < self.hash_count as u64 {
                Some(0)
            } else {
                Some(latest.saturating_sub(self.hash_count as u64 - 1))
            }
        } else {
            None
        }
    }

    /// Gets the hash at a given global index.
    /// Returns None if the index is outside the retained window [earliest_index, latest_index] (inclusive).
    pub fn get_hash_at_index(&self, index: u64) -> Option<&AnyHash> {
        if let Some(latest) = self.latest_index {
            let first = self.earliest_index().unwrap_or(0);
            if index < first || index > latest {
                return None;
            }
            let pos = (index % self.hash_count as u64) as usize;
            if pos < self.hashes.len() {
                Some(&self.hashes[pos])
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Returns all hashes in chronological order (oldest to newest).
    pub fn get_hashes_chronological(&self) -> Vec<AnyHash> {
        if let Some(latest) = self.latest_index {
            let first = self.earliest_index().unwrap_or(0);
            let mut result = Vec::new();
            for i in first..=latest {
                if let Some(hash) = self.get_hash_at_index(i) {
                    result.push(hash.clone());
                }
            }
            result
        } else {
            Vec::new()
        }
    }
}

impl PartialEq for OracleContract {
    fn eq(&self, other: &Self) -> bool {
        self.owner == other.owner
            && self.balance == other.balance
            && self.hash_count == other.hash_count
            && self.latest_index == other.latest_index
            && self.hashes == other.hashes
    }
}

impl Eq for OracleContract {}

impl PartialOrd for OracleContract {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OracleContract {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.owner
            .cmp(&other.owner)
            .then_with(|| self.balance.cmp(&other.balance))
            .then_with(|| self.hash_count.cmp(&other.hash_count))
            .then_with(|| self.latest_index.cmp(&other.latest_index))
            .then_with(|| self.hashes.cmp(&other.hashes))
    }
}
