use std::io::Write;

use nimiq_hash::{sha512::Sha512Hasher, Blake2bHasher, Hasher, Keccak256Hasher, Sha256Hasher};
use nimiq_keys::Address;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountType;
use nimiq_primitives::{account::AccountError, coin::Coin, transaction::TransactionError};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::account::htlc_contract::{AnyHash, AnyHash32, AnyHash64};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::account::oracle_contract::{
    CreationTransactionData, IncomingOracleTransactionData,
};
#[cfg(feature = "interaction-traits")]
use nimiq_transaction::{inherent::Inherent, SignatureProof, Transaction};

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
#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct OracleContract {
    /// The owner of the contract, the only address that can interact with it.
    pub owner: Address,

    /// When the contract is created, a deposit is required.
    /// If the balance goes to zero (effectively if the deposit is removed) the contract is deleted.
    /// We only support removing the deposit in full
    pub balance: Coin,

    /// The number of hashes that can be stored. This is set at creation and determines the required deposit.
    pub hash_count: u16,

    /// Fixed-size ring buffer of hashes. Always has exactly `hash_count` elements.
    /// Access entry at global index `i` via `hashes[i % hash_count]`.
    pub hashes: Vec<AnyHash>,

    /// Monotonically increasing counter tracking the total number of updates.
    /// The last valid index is `latest_index - 1` (if `latest_index > 0`).
    pub latest_index: u64,
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
        // Check any entry in the ring buffer
        for hash in &self.hashes {
            match hash {
                AnyHash::Blake2b(_) => return Some(HashType::Blake2b),
                AnyHash::Sha256(_) => return Some(HashType::Sha256),
                AnyHash::Sha512(_) => return Some(HashType::Sha512),
                AnyHash::Keccak256(_) => return Some(HashType::Keccak256),
            }
        }
        None
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
            HashType::from(&new_hashes[0])
        };

        // Validate all new hashes match the expected type
        if new_hashes
            .iter()
            .all(|hash| HashType::from(hash) == expected_type)
        {
            Ok(())
        } else {
            Err(AccountError::InvalidTransaction(
                TransactionError::InvalidData,
            ))
        }
    }
}

/// Helper enum to represent the hash type without the actual hash value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HashType {
    Blake2b,
    Sha256,
    Sha512,
    Keccak256,
}

impl HashType {
    fn from(hash: &AnyHash) -> Self {
        match hash {
            AnyHash::Blake2b(_) => HashType::Blake2b,
            AnyHash::Sha256(_) => HashType::Sha256,
            AnyHash::Sha512(_) => HashType::Sha512,
            AnyHash::Keccak256(_) => HashType::Keccak256,
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
            latest_index: 0,
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

                    // Get the zero hash type for chaining (using the type of the first new hash)
                    let zero_hash = if let Some(first_hash) = hashes.first() {
                        match first_hash {
                            AnyHash::Blake2b(_) => AnyHash::Blake2b(AnyHash32::default()),
                            AnyHash::Sha256(_) => AnyHash::Sha256(AnyHash32::default()),
                            AnyHash::Sha512(_) => AnyHash::Sha512(AnyHash64::default()),
                            AnyHash::Keccak256(_) => AnyHash::Keccak256(AnyHash32::default()),
                        }
                    } else if let Some(existing_hash) = self.hashes.iter().find(|h| {
                        // Find a non-zero hash
                        !matches!(h, AnyHash::Blake2b(h) if h.0 == [0u8; 32])
                    }) {
                        match existing_hash {
                            AnyHash::Blake2b(_) => AnyHash::Blake2b(AnyHash32::default()),
                            AnyHash::Sha256(_) => AnyHash::Sha256(AnyHash32::default()),
                            AnyHash::Sha512(_) => AnyHash::Sha512(AnyHash64::default()),
                            AnyHash::Keccak256(_) => AnyHash::Keccak256(AnyHash32::default()),
                        }
                    } else {
                        AnyHash::default()
                    };

                    // Ensure the ring buffer has at least hash_count capacity
                    // But only initialize entries as we write them (lazy initialization)
                    if self.hashes.capacity() < self.hash_count as usize {
                        self.hashes
                            .reserve_exact(self.hash_count as usize - self.hashes.capacity());
                    }

                    // Get the previous hash for chaining (latestData in Solidity)
                    let previous_hash = if self.latest_index > 0 {
                        let prev_pos = ((self.latest_index - 1) % self.hash_count as u64) as usize;
                        // Ensure the vec has enough entries
                        while self.hashes.len() <= prev_pos {
                            self.hashes.push(zero_hash.clone());
                        }
                        self.hashes[prev_pos].clone()
                    } else {
                        // First hash: use zero hash of the same type
                        zero_hash.clone()
                    };

                    // Implement ring buffer: write hashes at latest_index % hash_count
                    let mut removed_hashes = Vec::new();
                    let mut current_hash = previous_hash;

                    for new_hash in &hashes {
                        // Chain: data_i = H(data_{i-1} || state_i)
                        // Hash the previous chained hash and new hash together
                        current_hash = match new_hash {
                            AnyHash::Blake2b(_) => {
                                let mut hasher = Blake2bHasher::default();
                                hasher.write_all(current_hash.as_bytes()).unwrap();
                                hasher.write_all(new_hash.as_bytes()).unwrap();
                                AnyHash::Blake2b(AnyHash32(hasher.finish().into()))
                            }
                            AnyHash::Sha256(_) => {
                                let mut hasher = Sha256Hasher::default();
                                hasher.write_all(current_hash.as_bytes()).unwrap();
                                hasher.write_all(new_hash.as_bytes()).unwrap();
                                AnyHash::Sha256(AnyHash32(hasher.finish().into()))
                            }
                            AnyHash::Sha512(_) => {
                                let mut hasher = Sha512Hasher::default();
                                hasher.write_all(current_hash.as_bytes()).unwrap();
                                hasher.write_all(new_hash.as_bytes()).unwrap();
                                AnyHash::Sha512(AnyHash64(hasher.finish().into()))
                            }
                            AnyHash::Keccak256(_) => {
                                let mut hasher = Keccak256Hasher::default();
                                hasher.write_all(current_hash.as_bytes()).unwrap();
                                hasher.write_all(new_hash.as_bytes()).unwrap();
                                AnyHash::Keccak256(AnyHash32(hasher.finish().into()))
                            }
                        };

                        let pos = (self.latest_index % self.hash_count as u64) as usize;

                        // Ensure the vec has enough entries for this position
                        while self.hashes.len() <= pos {
                            self.hashes.push(zero_hash.clone());
                        }

                        // Save the hash that will be overwritten (for revert)
                        // Only save if we're overwriting (i.e., latest_index >= hash_count)
                        if self.latest_index >= self.hash_count as u64 {
                            removed_hashes.push(self.hashes[pos].clone());
                        }

                        // Write the chained hash at the ring buffer position
                        self.hashes[pos] = current_hash.clone();
                        self.latest_index += 1;
                    }

                    tx_logger.push_log(Log::OracleUpdate {
                        contract_address: transaction.recipient.clone(),
                        hashes,
                    });

                    // Return receipt with removed hashes for proper revert
                    if removed_hashes.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(UpdateReceipt { removed_hashes }.into()))
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
                    // Revert by restoring overwritten hashes and moving latest_index back
                    let num_hashes = hashes.len() as u64;

                    // Determine zero hash type for padding (if needed)
                    let zero_hash = if let Some(first_hash) = self.hashes.first() {
                        match first_hash {
                            AnyHash::Blake2b(_) => AnyHash::Blake2b(AnyHash32::default()),
                            AnyHash::Sha256(_) => AnyHash::Sha256(AnyHash32::default()),
                            AnyHash::Sha512(_) => AnyHash::Sha512(AnyHash64::default()),
                            AnyHash::Keccak256(_) => AnyHash::Keccak256(AnyHash32::default()),
                        }
                    } else {
                        AnyHash::default()
                    };

                    if let Some(receipt) = receipt {
                        let update_receipt = UpdateReceipt::try_from(receipt)?;

                        // Restore the overwritten hashes at their original positions
                        // The removed_hashes are saved in the order they were overwritten.
                        // When we wrote at latest_index, latest_index+1, ..., latest_index+num_hashes-1,
                        // we overwrote hashes at positions:
                        //   (latest_index) % hash_count,
                        //   (latest_index + 1) % hash_count,
                        //   ...
                        //   (latest_index + num_hashes - 1) % hash_count
                        // So to restore, we need to put them back at those same positions.
                        // But first, we need to move latest_index back.
                        let start_index = self.latest_index - num_hashes;
                        for (i, old_hash) in update_receipt.removed_hashes.iter().enumerate() {
                            let pos = ((start_index + i as u64) % self.hash_count as u64) as usize;
                            // Ensure the vec has enough entries
                            while self.hashes.len() <= pos {
                                self.hashes.push(zero_hash.clone());
                            }
                            self.hashes[pos] = old_hash.clone();
                        }
                    }

                    // Move latest_index back after restoring hashes
                    self.latest_index -= num_hashes;

                    // If latest_index is now 0, clear the ring buffer (no entries written yet)
                    if self.latest_index == 0 {
                        self.hashes.clear();
                    } else {
                        // The ring buffer should have exactly min(hash_count, latest_index) entries
                        // Once latest_index >= hash_count, we keep exactly hash_count entries
                        let target_len = if self.latest_index >= self.hash_count as u64 {
                            self.hash_count as usize
                        } else {
                            self.latest_index as usize
                        };
                        // Ensure the vector has the correct size
                        // If it's too short, pad with zero hashes (shouldn't happen, but be safe)
                        if self.hashes.len() < target_len {
                            while self.hashes.len() < target_len {
                                self.hashes.push(zero_hash.clone());
                            }
                        } else if self.hashes.len() > target_len {
                            // Trim if too long (shouldn't happen, but be safe)
                            self.hashes.truncate(target_len);
                        }
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
        // XXX This check should not be necessary since are also checking this in reserve_balance()
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
    pub latest_index: u64,
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
    pub fn earliest_index(&self) -> u64 {
        if self.latest_index <= self.hash_count as u64 {
            0
        } else {
            self.latest_index - self.hash_count as u64
        }
    }

    /// Gets the hash at a given global index.
    /// Returns None if the index is outside the retained window [earliest_index, latest_index).
    pub fn get_hash_at_index(&self, index: u64) -> Option<&AnyHash> {
        let first = self.earliest_index();
        if index < first || index >= self.latest_index {
            return None;
        }
        let pos = (index % self.hash_count as u64) as usize;
        if pos < self.hashes.len() {
            Some(&self.hashes[pos])
        } else {
            None
        }
    }

    /// Returns all hashes in chronological order (oldest to newest).
    pub fn get_hashes_chronological(&self) -> Vec<AnyHash> {
        let first = self.earliest_index();
        let mut result = Vec::new();
        for i in first..self.latest_index {
            if let Some(hash) = self.get_hash_at_index(i) {
                result.push(hash.clone());
            }
        }
        result
    }
}
