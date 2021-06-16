use std::convert::TryFrom;

use beserial::{Deserialize, Serialize};
use nimiq_keys::Address;
use nimiq_primitives::account::*;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::htlc_contract::{
    AnyHash, CreationTransactionData, HashAlgorithm, ProofType,
};
use nimiq_transaction::{SignatureProof, Transaction};

use crate::inherent::Inherent;
use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::{Account, AccountError};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
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
    pub fn new(
        balance: Coin,
        sender: Address,
        recipient: Address,
        hash_algorithm: HashAlgorithm,
        hash_root: AnyHash,
        hash_count: u8,
        timeout: u64,
        total_amount: Coin,
    ) -> Self {
        HashedTimeLockedContract {
            balance,
            sender,
            recipient,
            hash_algorithm,
            hash_root,
            hash_count,
            timeout,
            total_amount,
        }
    }

    pub fn with_balance(&self, balance: Coin) -> Self {
        HashedTimeLockedContract {
            balance,
            sender: self.sender.clone(),
            recipient: self.recipient.clone(),
            hash_algorithm: self.hash_algorithm,
            hash_root: self.hash_root.clone(),
            hash_count: self.hash_count,
            timeout: self.timeout,
            total_amount: self.total_amount,
        }
    }
}

impl AccountTransactionInteraction for HashedTimeLockedContract {
    fn new_contract(
        account_type: AccountType,
        balance: Coin,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Self, AccountError> {
        if account_type == AccountType::HTLC {
            HashedTimeLockedContract::create(balance, transaction, block_height, time)
        } else {
            Err(AccountError::InvalidForRecipient)
        }
    }

    fn create(
        balance: Coin,
        transaction: &Transaction,
        _block_height: u32,
        _time: u64,
    ) -> Result<Self, AccountError> {
        let data = CreationTransactionData::parse(transaction)?;
        Ok(HashedTimeLockedContract::new(
            balance,
            data.sender,
            data.recipient,
            data.hash_algorithm,
            data.hash_root,
            data.hash_count,
            data.timeout,
            transaction.value,
        ))
    }

    fn check_incoming_transaction(
        _transaction: &Transaction,
        _block_height: u32,
        _time: u64,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_height: u32,
        _time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_incoming_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_height: u32,
        _time: u64,
        _receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn check_outgoing_transaction(
        &self,
        transaction: &Transaction,
        _block_height: u32,
        time: u64,
    ) -> Result<(), AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.total_value()?)?;
        let proof_buf = &mut &transaction.proof[..];
        let proof_type: ProofType = Deserialize::deserialize(proof_buf)?;
        match proof_type {
            ProofType::RegularTransfer => {
                // Check that the contract has not expired yet.
                if self.timeout < time {
                    warn!("HTLC has expired: {} < {}", self.timeout, time);
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
                if balance < min_cap {
                    return Err(AccountError::InsufficientFunds {
                        balance,
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
                if self.timeout >= time {
                    warn!("HTLC has not yet expired: {} >= {}", self.timeout, time);
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the transaction is signed by the original sender.
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;
                if !signature_proof.is_signed_by(&self.sender) {
                    return Err(AccountError::InvalidSignature);
                }
            }
        }

        Ok(())
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        self.check_outgoing_transaction(transaction, block_height, time)?;
        self.balance = Account::balance_sub(self.balance, transaction.total_value()?)?;
        Ok(None)
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_height: u32,
        _time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidReceipt);
        }

        self.balance = Account::balance_add(self.balance, transaction.total_value()?)?;
        Ok(())
    }
}

impl AccountInherentInteraction for HashedTimeLockedContract {
    fn check_inherent(
        &self,
        _inherent: &Inherent,
        _block_height: u32,
        _time: u64,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidInherent)
    }

    fn commit_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_height: u32,
        _time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        Err(AccountError::InvalidInherent)
    }

    fn revert_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_height: u32,
        _time: u64,
        _receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidInherent)
    }
}
