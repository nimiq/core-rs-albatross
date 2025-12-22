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

    /// The stored hashes. The number of hashes must not exceed hash_count.
    pub hashes: Vec<AnyHash>,
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

    /// Gets the hash type from the first hash in the contract, if any.
    /// Returns None if the contract has no hashes yet.
    fn get_hash_type(&self) -> Option<HashType> {
        self.hashes.first().map(HashType::from)
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
            hashes: Vec::new(),
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

                    // Implement ring buffer: remove oldest hashes if we would exceed hash_count
                    let new_hash_count = self.hashes.len() + hashes.len();
                    let removed_hashes = if new_hash_count > self.hash_count as usize {
                        // Remove the oldest hashes to make room for new ones
                        let hashes_to_remove = new_hash_count - self.hash_count as usize;
                        let removed: Vec<AnyHash> =
                            self.hashes.drain(0..hashes_to_remove).collect();
                        removed
                    } else {
                        Vec::new()
                    };

                    // Store the previous hash state for revert
                    let previous_hash_state = self.hashes.last().cloned();

                    // Add the new hashes with chaining: hash_i = hash((hash_{i-1}, hash_i))
                    for new_hash in &hashes {
                        // Get the previous hash (last in the list, or zero hash if empty)
                        let previous_hash = self.hashes.last().cloned().unwrap_or_else(|| {
                            // Use a zero hash of the same type as the new hash
                            new_hash.zero()
                        });

                        // Hash the previous hash and new hash using the same algorithm as the new hash
                        // Chain: data_i = H(data_{i-1} || state_i) = H(previous_hash || new_hash)
                        let chained_hash = previous_hash.digest(new_hash);

                        // Store the chained hash
                        self.hashes.push(chained_hash);
                    }

                    tx_logger.push_log(Log::OracleUpdate {
                        contract_address: transaction.recipient.clone(),
                        hashes,
                    });

                    // Return receipt with removed hashes and previous hash state for proper revert
                    // Only return a receipt if we removed hashes or if we need to track previous state
                    if removed_hashes.is_empty() && previous_hash_state.is_none() {
                        Ok(None)
                    } else {
                        Ok(Some(
                            UpdateReceipt {
                                removed_hashes,
                                previous_hash_state,
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
                    // Remove the chained hashes that were added (they're at the end)
                    for _ in 0..hashes.len() {
                        self.hashes.pop();
                    }

                    // Restore the hashes that were removed (if any)
                    if let Some(receipt) = receipt {
                        let update_receipt = UpdateReceipt::try_from(receipt)?;
                        // Prepend the removed hashes back to the beginning
                        let mut restored_hashes = update_receipt.removed_hashes;
                        restored_hashes.append(&mut self.hashes);
                        self.hashes = restored_hashes;
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
}

impl From<OracleContract> for PrunedOracleContract {
    fn from(contract: OracleContract) -> Self {
        PrunedOracleContract {
            owner: contract.owner,
            hash_count: contract.hash_count,
            hashes: contract.hashes,
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
    /// The previous hash state before this update (for proper revert of chained hashes)
    pub previous_hash_state: Option<AnyHash>,
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
