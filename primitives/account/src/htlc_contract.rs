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
use crate::logs::{AccountInfo, Log};
use crate::{Account, AccountError, AccountsTrie, BasicAccount};

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

    #[must_use]
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

    pub fn can_change_balance(
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
    fn create(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<AccountInfo, AccountError> {
        let data = CreationTransactionData::parse(transaction)?;

        let contract_key = KeyNibbles::from(&transaction.contract_creation_address());

        let previous_balance = match accounts_tree.get(db_txn, &contract_key) {
            None => Coin::ZERO,
            Some(account) => account.balance(),
        };

        let contract = HashedTimeLockedContract::new(
            previous_balance + transaction.value,
            data.sender.clone(),
            data.recipient.clone(),
            data.hash_algorithm,
            data.hash_root.clone(),
            data.hash_count,
            data.timeout,
            transaction.value,
        );

        accounts_tree.put(db_txn, &contract_key, Account::HTLC(contract.clone()));
        let logs = vec![Log::HTLCCreate {
            contract_address: transaction.recipient.clone(),
            sender: contract.sender,
            recipient: contract.recipient,
            hash_algorithm: contract.hash_algorithm,
            hash_root: contract.hash_root,
            hash_count: contract.hash_count,
            timeout: contract.timeout,
            total_amount: transaction.value,
        }];

        Ok(AccountInfo::new(None, logs))
    }

    fn commit_incoming_transaction(
        _accounts_tree: &AccountsTrie,
        _db_txn: &mut WriteTransaction,
        _transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<AccountInfo, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_incoming_transaction(
        _accounts_tree: &AccountsTrie,
        _db_txn: &mut WriteTransaction,
        _transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        _receipt: Option<&Vec<u8>>,
    ) -> Result<Vec<Log>, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_outgoing_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        block_time: u64,
    ) -> Result<AccountInfo, AccountError> {
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

        let new_balance = Account::balance_sub(account.balance(), transaction.total_value())?;

        htlc.can_change_balance(transaction.proof.clone(), new_balance, block_time)?;

        let proof_buf = &mut &transaction.proof[..];

        let proof_type: ProofType = Deserialize::deserialize(proof_buf)?;

        // Create the contract logs
        let mut logs = vec![
            Log::PayFee {
                from: transaction.sender.clone(),
                fee: transaction.fee,
            },
            Log::Transfer {
                from: transaction.sender.clone(),
                to: transaction.recipient.clone(),
                amount: transaction.value,
            },
        ];

        match proof_type {
            ProofType::RegularTransfer => {
                // Ignore pre_image.
                let pre_image: AnyHash = Deserialize::deserialize(proof_buf)?;
                let hash_depth: u8 = Deserialize::deserialize(proof_buf)?;

                logs.push(Log::HTLCRegularTransfer {
                    contract_address: transaction.sender.clone(),
                    pre_image,
                    hash_depth,
                });
            }
            ProofType::EarlyResolve => {
                logs.push(Log::HTLCEarlyResolve {
                    contract_address: transaction.sender.clone(),
                });
            }
            ProofType::TimeoutResolve => {
                logs.push(Log::HTLCTimeoutResolve {
                    contract_address: transaction.sender.clone(),
                });
            }
        }

        // Store the account or prune if necessary.
        let receipt = if new_balance.is_zero() {
            accounts_tree.remove(db_txn, &key);

            Some(HTLCReceipt::from(htlc.clone()).serialize_to_vec())
        } else {
            accounts_tree.put(
                db_txn,
                &key,
                Account::HTLC(htlc.change_balance(new_balance)),
            );

            None
        };

        Ok(AccountInfo::new(receipt, logs))
    }

    fn revert_outgoing_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<Vec<Log>, AccountError> {
        let key = KeyNibbles::from(&transaction.sender);

        let htlc = match receipt {
            None => {
                let account =
                    accounts_tree
                        .get(db_txn, &key)
                        .ok_or(AccountError::NonExistentAddress {
                            address: transaction.sender.clone(),
                        })?;

                if let Account::HTLC(contract) = account {
                    contract
                } else {
                    return Err(AccountError::TypeMismatch {
                        expected: AccountType::HTLC,
                        got: account.account_type(),
                    });
                }
            }
            Some(r) => {
                let receipt: HTLCReceipt = Deserialize::deserialize_from_vec(r)?;

                HashedTimeLockedContract::from(receipt)
            }
        };

        let new_balance = Account::balance_add(htlc.balance, transaction.total_value())?;

        accounts_tree.put(
            db_txn,
            &key,
            Account::HTLC(htlc.change_balance(new_balance)),
        );

        // Build the revert logs
        let mut logs = vec![
            Log::PayFee {
                from: transaction.sender.clone(),
                fee: transaction.fee,
            },
            Log::Transfer {
                from: transaction.sender.clone(),
                to: transaction.recipient.clone(),
                amount: transaction.value,
            },
        ];

        let proof_buf = &mut &transaction.proof[..];
        let proof_type: ProofType = Deserialize::deserialize(proof_buf)?;

        logs.push(match proof_type {
            ProofType::RegularTransfer => {
                let pre_image: AnyHash = Deserialize::deserialize(proof_buf)?;
                let hash_depth: u8 = Deserialize::deserialize(proof_buf)?;
                Log::HTLCRegularTransfer {
                    contract_address: htlc.sender,
                    pre_image,
                    hash_depth,
                }
            }
            ProofType::EarlyResolve => Log::HTLCEarlyResolve {
                contract_address: htlc.sender,
            },
            ProofType::TimeoutResolve => Log::HTLCTimeoutResolve {
                contract_address: htlc.sender,
            },
        });
        Ok(logs)
    }

    fn commit_failed_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
    ) -> Result<AccountInfo, AccountError> {
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

        // Note that in this type of transactions the fee is paid (deducted) from the contract balance
        let new_balance = Account::balance_sub(account.balance(), transaction.fee)?;

        let logs = vec![Log::PayFee {
            from: transaction.sender.clone(),
            fee: transaction.fee,
        }];

        // Store the account or prune if necessary.
        let receipt = if new_balance.is_zero() {
            accounts_tree.remove(db_txn, &key);

            Some(HTLCReceipt::from(htlc.clone()).serialize_to_vec())
        } else {
            accounts_tree.put(
                db_txn,
                &key,
                Account::HTLC(htlc.change_balance(new_balance)),
            );

            None
        };

        Ok(AccountInfo::new(receipt, logs))
    }

    fn revert_failed_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        receipt: Option<&Vec<u8>>,
    ) -> Result<Vec<Log>, AccountError> {
        let key = KeyNibbles::from(&transaction.sender);

        let htlc = match receipt {
            None => {
                let account =
                    accounts_tree
                        .get(db_txn, &key)
                        .ok_or(AccountError::NonExistentAddress {
                            address: transaction.sender.clone(),
                        })?;

                if let Account::HTLC(contract) = account {
                    contract
                } else {
                    return Err(AccountError::TypeMismatch {
                        expected: AccountType::HTLC,
                        got: account.account_type(),
                    });
                }
            }
            Some(r) => {
                let receipt: HTLCReceipt = Deserialize::deserialize_from_vec(r)?;

                HashedTimeLockedContract::from(receipt)
            }
        };

        let new_balance = Account::balance_add(htlc.balance, transaction.fee)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account::HTLC(htlc.change_balance(new_balance)),
        );

        // Build the revert logs
        let logs = vec![Log::PayFee {
            from: transaction.sender.clone(),
            fee: transaction.fee,
        }];

        Ok(logs)
    }

    fn can_pay_fee(
        &self,
        transaction: &Transaction,
        mempool_balance: Coin,
        block_time: u64,
    ) -> bool {
        let new_balance = match Account::balance_sub(self.balance, mempool_balance) {
            Ok(new_balance) => new_balance,
            Err(_) => {
                return false;
            }
        };

        self.can_change_balance(transaction.proof.clone(), new_balance, block_time)
            .is_ok()
    }

    fn delete(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
    ) -> Result<Vec<Log>, AccountError> {
        let key = KeyNibbles::from(&transaction.contract_creation_address());

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.sender.clone(),
            })?;

        let htlc = match account {
            Account::HTLC(ref value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Vesting,
                    got: account.account_type(),
                })
            }
        };

        let previous_balance = Account::balance_sub(htlc.balance, transaction.value)?;

        if previous_balance == Coin::ZERO {
            // If the previous balance was zero, we just remove the account from the accounts tree
            accounts_tree.remove(db_txn, &key);
        } else {
            // If the previous balance was not zero, we need to restore the basic account with the previous balance
            accounts_tree.put(
                db_txn,
                &key,
                Account::Basic(BasicAccount {
                    balance: previous_balance,
                }),
            );
        }
        Ok(Vec::new())
    }
}

impl AccountInherentInteraction for HashedTimeLockedContract {
    fn commit_inherent(
        _accounts_tree: &AccountsTrie,
        _db_txn: &mut WriteTransaction,
        _inherent: &Inherent,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<AccountInfo, AccountError> {
        Err(AccountError::InvalidInherent)
    }

    fn revert_inherent(
        _accounts_tree: &AccountsTrie,
        _db_txn: &mut WriteTransaction,
        _inherent: &Inherent,
        _block_height: u32,
        _block_time: u64,
        _receipt: Option<&Vec<u8>>,
    ) -> Result<Vec<Log>, AccountError> {
        Err(AccountError::InvalidInherent)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct HTLCReceipt {
    pub sender: Address,
    pub recipient: Address,
    pub hash_algorithm: HashAlgorithm,
    pub hash_root: AnyHash,
    pub hash_count: u8,
    pub timeout: u64,
    pub total_amount: Coin,
}

impl From<HashedTimeLockedContract> for HTLCReceipt {
    fn from(contract: HashedTimeLockedContract) -> Self {
        HTLCReceipt {
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

impl From<HTLCReceipt> for HashedTimeLockedContract {
    fn from(receipt: HTLCReceipt) -> Self {
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
