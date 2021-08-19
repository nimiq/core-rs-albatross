use std::convert::TryFrom;

use beserial::{Deserialize, Serialize};
use nimiq_database::WriteTransaction;
use nimiq_keys::Address;
use nimiq_primitives::account::*;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::htlc_contract::{
    AnyHash, CreationTransactionData, HashAlgorithm, ProofType,
};
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_trie::key_nibbles::KeyNibbles;

use crate::inherent::Inherent;
use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::{Account, AccountError, AccountsTrie};

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

    pub fn change_balance(&self, balance: Coin) -> Self {
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
    fn create(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<(), AccountError> {
        let data = CreationTransactionData::parse(transaction)?;

        let contract_key = KeyNibbles::from(&transaction.contract_creation_address());

        let previous_balance = match accounts_tree.get(db_txn, &contract_key) {
            None => Coin::ZERO,
            Some(account) => account.balance(),
        };

        let contract = HashedTimeLockedContract::new(
            previous_balance + transaction.value,
            data.sender,
            data.recipient,
            data.hash_algorithm,
            data.hash_root,
            data.hash_count,
            data.timeout,
            transaction.value,
        );

        accounts_tree.put(db_txn, &contract_key, Account::HTLC(contract));

        Ok(())
    }

    fn commit_incoming_transaction(
        _accounts_tree: &AccountsTrie,
        _db_txn: &mut WriteTransaction,
        _transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_incoming_transaction(
        _accounts_tree: &AccountsTrie,
        _db_txn: &mut WriteTransaction,
        _transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        _receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_outgoing_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        let key = KeyNibbles::from(&transaction.sender);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.sender.clone(),
            })?;

        let htlc = match account {
            Account::HTLC(ref value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::HTLC,
                    got: account.account_type(),
                })
            }
        };

        let new_balance = Account::balance_sub(account.balance(), transaction.total_value()?)?;

        let proof_buf = &mut &transaction.proof[..];

        let proof_type: ProofType = Deserialize::deserialize(proof_buf)?;

        match proof_type {
            ProofType::RegularTransfer => {
                // Check that the contract has not expired yet.
                if htlc.timeout < block_time {
                    warn!("HTLC has expired: {} < {}", htlc.timeout, block_time);
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the provided hash_root is correct.
                let hash_algorithm: HashAlgorithm = Deserialize::deserialize(proof_buf)?;

                let hash_depth: u8 = Deserialize::deserialize(proof_buf)?;

                let hash_root: AnyHash = Deserialize::deserialize(proof_buf)?;

                if hash_algorithm != htlc.hash_algorithm || hash_root != htlc.hash_root {
                    warn!("HTLC hash mismatch");
                    return Err(AccountError::InvalidForSender);
                }

                // Ignore pre_image.
                let _pre_image: AnyHash = Deserialize::deserialize(proof_buf)?;

                // Check that the transaction is signed by the authorized recipient.
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if !signature_proof.is_signed_by(&htlc.recipient) {
                    return Err(AccountError::InvalidSignature);
                }

                // Check min cap.
                let cap_ratio = 1f64 - (f64::from(hash_depth) / f64::from(htlc.hash_count));

                let min_cap = Coin::try_from(
                    (cap_ratio * u64::from(htlc.total_amount) as f64)
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

                if !signature_proof_recipient.is_signed_by(&htlc.recipient)
                    || !signature_proof_sender.is_signed_by(&htlc.sender)
                {
                    return Err(AccountError::InvalidSignature);
                }
            }
            ProofType::TimeoutResolve => {
                // Check that the contract has expired.
                if htlc.timeout >= block_time {
                    warn!(
                        "HTLC has not yet expired: {} >= {}",
                        htlc.timeout, block_time
                    );
                    return Err(AccountError::InvalidForSender);
                }

                // Check that the transaction is signed by the original sender.
                let signature_proof: SignatureProof = Deserialize::deserialize(proof_buf)?;

                if !signature_proof.is_signed_by(&htlc.sender) {
                    return Err(AccountError::InvalidSignature);
                }
            }
        }

        accounts_tree.put(
            db_txn,
            &key,
            Account::HTLC(htlc.change_balance(new_balance)),
        );

        Ok(None)
    }

    fn revert_outgoing_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidReceipt);
        }

        let key = KeyNibbles::from(&transaction.sender);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.sender.clone(),
            })?;

        let htlc = match account {
            Account::HTLC(ref value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::HTLC,
                    got: account.account_type(),
                })
            }
        };

        let new_balance = Account::balance_add(account.balance(), transaction.total_value()?)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account::HTLC(htlc.change_balance(new_balance)),
        );

        Ok(())
    }
}

impl AccountInherentInteraction for HashedTimeLockedContract {
    fn commit_inherent(
        _accounts_tree: &AccountsTrie,
        _db_txn: &mut WriteTransaction,
        _inherent: &Inherent,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        Err(AccountError::InvalidInherent)
    }

    fn revert_inherent(
        _accounts_tree: &AccountsTrie,
        _db_txn: &mut WriteTransaction,
        _inherent: &Inherent,
        _block_height: u32,
        _block_time: u64,
        _receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidInherent)
    }
}
