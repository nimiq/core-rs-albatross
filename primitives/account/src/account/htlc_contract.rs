use beserial::{Deserialize, Serialize};
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::htlc_contract::{
    AnyHash, CreationTransactionData, HashAlgorithm, ProofType,
};
use nimiq_transaction::{SignatureProof, Transaction};

use crate::data_store::{DataStoreRead, DataStoreWrite};
use crate::inherent::Inherent;
use crate::interaction_traits::{
    AccountInherentInteraction, AccountPruningInteraction, AccountTransactionInteraction,
};
use crate::{Account, AccountError, AccountReceipt};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
pub struct HashedTimeLockedContract {
    pub balance: Coin,
    pub sender: Address,
    pub recipient: Address,
    pub hash_algorithm: HashAlgorithm,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    pub timeout: u64,
    pub total_amount: Coin,
}

impl HashedTimeLockedContract {
    fn can_change_balance(
        &self,
        proof: Vec<u8>,
        new_balance: Coin,
        block_time: u64,
    ) -> Result<bool, AccountError> {
        let proof_buf = &mut &proof[..];
        let proof_type: ProofType = Deserialize::deserialize(proof_buf)?;

        match proof_type {
            ProofType::RegularTransfer => {
                // Check that the contract has not expired yet.
                if self.timeout < block_time {
                    warn!("HTLC has expired: {} < {}", self.timeout, block_time);
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the provided hash_root is correct.
                let hash_algorithm: HashAlgorithm = Deserialize::deserialize(proof_buf)?;

                let hash_depth: u8 = Deserialize::deserialize(proof_buf)?;

                let hash_root: AnyHash = Deserialize::deserialize(proof_buf)?;

                if hash_algorithm != self.hash_algorithm || hash_root != self.hash_root {
                    warn!("HTLC hash mismatch");
                    return Err(AccountError::InvalidForSender);
                }

                // Ignore pre_image.
                let _pre_image: AnyHash = Deserialize::deserialize(proof_buf)?;

                // Check that the transaction is signed by the authorized recipient.
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if !signature_proof.is_signed_by(&self.recipient) {
                    return Err(AccountError::InvalidSignature);
                }

                // Check min cap.
                let cap_ratio = 1f64 - (f64::from(hash_depth) / f64::from(self.hash_count));

                let min_cap = Coin::try_from(
                    (cap_ratio * u64::from(self.total_amount) as f64)
                        .floor()
                        .max(0f64) as u64,
                )?;

                if new_balance < min_cap {
                    return Err(AccountError::InsufficientFunds {
                        balance: new_balance,
                        needed: min_cap,
                    });
                }
            }
            ProofType::EarlyResolve => {
                // Check that the transaction is signed by both parties.
                let signature_proof_recipient: SignatureProof =
                    Deserialize::deserialize(proof_buf)?;

                let signature_proof_sender: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if !signature_proof_recipient.is_signed_by(&self.recipient)
                    || !signature_proof_sender.is_signed_by(&self.sender)
                {
                    return Err(AccountError::InvalidSignature);
                }
            }
            ProofType::TimeoutResolve => {
                // Check that the contract has expired.
                if self.timeout >= block_time {
                    warn!(
                        "HTLC has not yet expired: {} >= {}",
                        self.timeout, block_time
                    );
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the transaction is signed by the original sender.
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if !signature_proof.is_signed_by(&self.sender) {
                    return Err(AccountError::InvalidSignature);
                }
            }
        };

        Ok(true)
    }
}

impl AccountTransactionInteraction for HashedTimeLockedContract {
    fn create_new_contract(
        transaction: &Transaction,
        initial_balance: Coin,
        _block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        let data = CreationTransactionData::parse(transaction)?;
        Ok(Account::HTLC(HashedTimeLockedContract {
            balance: Account::balance_add(initial_balance, transaction.value)?,
            sender: data.sender,
            recipient: data.recipient,
            hash_algorithm: data.hash_algorithm,
            hash_root: data.hash_root,
            hash_count: data.hash_count,
            timeout: data.timeout,
            total_amount: transaction.value,
        }))
    }

    fn revert_new_contract(
        &mut self,
        transaction: &Transaction,
        _block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        Account::balance_sub_assign(&mut self.balance, transaction.value)
    }

    fn commit_incoming_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_incoming_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_time: u64,
        _receipt: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let new_balance = Account::balance_sub(self.balance, transaction.total_value())?;
        self.can_change_balance(transaction.proof.clone(), new_balance, block_time)?;
        self.balance = new_balance;
        Ok(None)
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_time: u64,
        _receipt: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        Account::balance_add_assign(&mut self.balance, transaction.total_value())
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let new_balance = Account::balance_sub(self.balance, transaction.fee)?;
        // XXX This check should not be necessary since are also checking this in has_sufficient_balance()
        self.can_change_balance(transaction.proof.clone(), new_balance, block_time)?;
        self.balance = new_balance;
        Ok(None)
    }

    fn revert_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_time: u64,
        _receipt: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        Account::balance_add_assign(&mut self.balance, transaction.fee)
    }

    fn has_sufficient_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: Coin,
        block_time: u64,
        _data_store: DataStoreRead,
    ) -> Result<bool, AccountError> {
        let needed = reserved_balance
            .checked_add(transaction.total_value())
            .ok_or(AccountError::InvalidCoinValue)?;
        if self.balance < needed {
            return Ok(false);
        }

        // XXX We could also assert here (or not use checked_sub) since we know that balance >= needed.
        let new_balance = self
            .balance
            .checked_sub(needed)
            .ok_or(AccountError::InvalidCoinValue)?;
        self.can_change_balance(transaction.proof.clone(), new_balance, block_time)
    }
}

impl AccountInherentInteraction for HashedTimeLockedContract {
    fn commit_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_time: u64,
        _data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        Err(AccountError::InvalidForTarget)
    }

    fn revert_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_time: u64,
        _receipt: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForTarget)
    }
}

impl AccountPruningInteraction for HashedTimeLockedContract {
    fn can_be_pruned(&self) -> bool {
        self.balance.is_zero()
    }

    fn prune(self, _data_store: DataStoreRead) -> Result<Option<AccountReceipt>, AccountError> {
        Ok(Some(PrunedHashedTimeLockContract::from(self).into()))
    }

    fn restore(
        _ty: AccountType,
        pruned_account: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        let receipt = pruned_account.ok_or(AccountError::InvalidReceipt)?;
        Ok(Account::HTLC(HashedTimeLockedContract::from(
            receipt.into(),
        )))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrunedHashedTimeLockContract {
    pub sender: Address,
    pub recipient: Address,
    pub hash_algorithm: HashAlgorithm,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    pub timeout: u64,
    pub total_amount: Coin,
}

impl From<HashedTimeLockedContract> for PrunedHashedTimeLockContract {
    fn from(contract: HashedTimeLockedContract) -> Self {
        PrunedHashedTimeLockContract {
            sender: contract.sender,
            recipient: contract.recipient,
            hash_algorithm: contract.hash_algorithm,
            hash_root: contract.hash_root,
            hash_count: contract.hash_count,
            timeout: contract.timeout,
            total_amount: contract.total_amount,
        }
    }
}

impl From<PrunedHashedTimeLockContract> for HashedTimeLockedContract {
    fn from(receipt: PrunedHashedTimeLockContract) -> Self {
        HashedTimeLockedContract {
            balance: Coin::ZERO,
            sender: receipt.sender,
            recipient: receipt.recipient,
            hash_algorithm: receipt.hash_algorithm,
            hash_root: receipt.hash_root,
            hash_count: receipt.hash_count,
            timeout: receipt.timeout,
            total_amount: receipt.total_amount,
        }
    }
}
