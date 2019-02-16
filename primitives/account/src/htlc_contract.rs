use beserial::{Deserialize, Serialize};
use keys::Address;
use primitives::account::*;
pub use primitives::account::{AnyHash, HashAlgorithm, ProofType};
use primitives::coin::Coin;
use transaction::{SignatureProof, Transaction};
use transaction::account::parse_and_verify_htlc_creation_transaction;

use crate::{Account, AccountError};
use crate::AccountTransactionInteraction;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct HashedTimeLockedContract {
    pub balance: Coin,
    pub sender: Address,
    pub recipient: Address,
    pub hash_algorithm: HashAlgorithm,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    pub timeout: u32,
    pub total_amount: Coin
}

impl HashedTimeLockedContract {
    pub fn new(balance: Coin, sender: Address, recipient: Address, hash_algorithm: HashAlgorithm, hash_root: AnyHash, hash_count: u8, timeout: u32, total_amount: Coin) -> Self {
        return HashedTimeLockedContract { balance, sender, recipient, hash_algorithm, hash_root, hash_count, timeout, total_amount };
    }

    pub fn with_balance(&self, balance: Coin) -> Self {
        return HashedTimeLockedContract {
            balance,
            sender: self.sender.clone(),
            recipient: self.recipient.clone(),
            hash_algorithm: self.hash_algorithm,
            hash_root: self.hash_root.clone(),
            hash_count: self.hash_count,
            timeout: self.timeout,
            total_amount: self.total_amount,
        };
    }
}

impl AccountTransactionInteraction for HashedTimeLockedContract {
    fn new_contract(account_type: AccountType, balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        if account_type == AccountType::HTLC {
            HashedTimeLockedContract::create(balance, transaction, block_height)
        } else {
            Err(AccountError::InvalidForRecipient)
        }
    }

    fn create(balance: Coin, transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        let (sender, recipient, hash_algorithm, hash_root, hash_count, timeout) = parse_and_verify_htlc_creation_transaction(transaction)?;
        return Ok(HashedTimeLockedContract::new(balance, sender, recipient, hash_algorithm, hash_root, hash_count, timeout, transaction.value));
    }

    fn with_incoming_transaction(&self, _transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError::InvalidForRecipient);
    }

    fn without_incoming_transaction(&self, _transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError::InvalidForRecipient);
    }

    fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.value.checked_add(transaction.fee).ok_or(AccountError::InvalidCoinValue)?)?;
        let proof_buf = &mut &transaction.proof[..];
        let proof_type: ProofType = Deserialize::deserialize(proof_buf)?;
        match proof_type {
            ProofType::RegularTransfer => {
                // Check that the contract has not expired yet.
                if self.timeout < block_height {
                    warn!("HTLC expired: {} < {}", self.timeout, block_height);
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
                let cap_ratio = 1f64 - (hash_depth as f64 / self.hash_count as f64);
                let min_cap = (cap_ratio * u64::from(self.total_amount) as f64).floor().max(0f64) as u64;
                if balance < Coin::from_u64(min_cap)? {
                    return Err(AccountError::InsufficientFunds);
                }
            },
            ProofType::EarlyResolve => {
                // Check that the transaction is signed by both parties.
                let signature_proof_recipient: SignatureProof = Deserialize::deserialize(proof_buf)?;
                let signature_proof_sender: SignatureProof = Deserialize::deserialize(proof_buf)?;
                if !signature_proof_recipient.is_signed_by(&self.recipient)
                    || !signature_proof_sender.is_signed_by(&self.sender) {
                    return Err(AccountError::InvalidSignature);
                }
            },
            ProofType::TimeoutResolve => {
                // Check that the contract has expired.
                if self.timeout >= block_height {
                    warn!("HTLC not yet expired: {} >= {}", self.timeout, block_height);
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the transaction is signed by the original sender.
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;
                if !signature_proof.is_signed_by(&self.sender) {
                    return Err(AccountError::InvalidSignature);
                }
            }
        }
        Ok(self.with_balance(balance))
    }

    fn without_outgoing_transaction(&self, transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_add(self.balance, transaction.value.checked_add(transaction.fee).ok_or(AccountError::InvalidCoinValue)?)?;
        return Ok(self.with_balance(balance));
    }
}
