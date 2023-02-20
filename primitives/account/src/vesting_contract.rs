use beserial::{Deserialize, Serialize};
use nimiq_database::WriteTransaction;
use nimiq_keys::Address;
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
    key_nibbles::KeyNibbles,
};
use nimiq_transaction::{
    account::vesting_contract::CreationTransactionData, inherent::Inherent, SignatureProof,
    Transaction,
};

use crate::{
    complete, get_or_update_account,
    interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction},
    logs::{AccountInfo, Log},
    Account, AccountsTrie, BasicAccount,
};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde-derive", serde(rename_all = "camelCase"))]
pub struct VestingContract {
    pub balance: Coin,
    pub owner: Address,
    pub start_time: u64,
    pub time_step: u64,
    pub step_amount: Coin,
    pub total_amount: Coin,
}

impl VestingContract {
    pub fn new(
        balance: Coin,
        owner: Address,
        start_time: u64,
        time_step: u64,
        step_amount: Coin,
        total_amount: Coin,
    ) -> Self {
        VestingContract {
            balance,
            owner,
            start_time,
            time_step,
            step_amount,
            total_amount,
        }
    }

    #[must_use]
    pub fn change_balance(&self, balance: Coin) -> Self {
        VestingContract {
            balance,
            owner: self.owner.clone(),
            start_time: self.start_time,
            time_step: self.time_step,
            step_amount: self.step_amount,
            total_amount: self.total_amount,
        }
    }

    pub fn min_cap(&self, time: u64) -> Coin {
        if self.time_step > 0 && self.step_amount > Coin::ZERO {
            let steps = (time as i128 - self.start_time as i128) / self.time_step as i128;
            let min_cap =
                u64::from(self.total_amount) as i128 - steps * u64::from(self.step_amount) as i128;
            // Since all parameters have been validated, this will be safe as well.
            Coin::from_u64_unchecked(min_cap.max(0) as u64)
        } else {
            Coin::ZERO
        }
    }
}

impl AccountTransactionInteraction for VestingContract {
    fn create(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<AccountInfo, AccountError> {
        let data = CreationTransactionData::parse(transaction)?;

        let contract_key = KeyNibbles::from(&transaction.contract_creation_address());

        let previous_balance = match complete!(get_or_update_account::<Account>(
            accounts_tree,
            db_txn,
            &contract_key
        )) {
            None => Coin::ZERO,
            Some(account) => account.balance(),
        };

        let contract = VestingContract::new(
            previous_balance + transaction.value,
            data.owner,
            data.start_time,
            data.time_step,
            data.step_amount,
            data.total_amount,
        );

        accounts_tree
            .put(db_txn, &contract_key, Account::Vesting(contract.clone()))
            .expect("temporary until accounts rewrite");

        let logs = vec![Log::VestingCreate {
            contract_address: transaction.recipient.clone(),
            owner: contract.owner,
            start_time: contract.start_time,
            time_step: contract.time_step,
            step_amount: contract.step_amount,
            total_amount: contract.total_amount,
        }];
        Ok(AccountInfo::with_receipt(None, logs))
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

        let account = match complete!(get_or_update_account::<Account>(
            accounts_tree,
            db_txn,
            &key
        )) {
            Some(account) => account,
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: transaction.sender.clone(),
                });
            }
        };

        let vesting = match account {
            Account::Vesting(ref value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Vesting,
                    got: account.account_type(),
                })
            }
        };

        let new_balance = Account::balance_sub(account.balance(), transaction.total_value())?;

        // Check vesting min cap.
        let min_cap = vesting.min_cap(block_time);

        if new_balance < min_cap {
            return Err(AccountError::InsufficientFunds {
                balance: new_balance,
                needed: min_cap,
            });
        }

        // Check transaction signer is contract owner.
        let signature_proof: SignatureProof =
            Deserialize::deserialize(&mut &transaction.proof[..])?;

        if !signature_proof.is_signed_by(&vesting.owner) {
            return Err(AccountError::InvalidSignature);
        }

        // Store the account or prune if necessary.
        let receipt = if new_balance.is_zero() {
            accounts_tree.remove(db_txn, &key);

            Some(VestingReceipt::from(vesting.clone()).serialize_to_vec())
        } else {
            accounts_tree
                .put(
                    db_txn,
                    &key,
                    Account::Vesting(vesting.change_balance(new_balance)),
                )
                .expect("temporary until accounts rewrite");

            None
        };
        let logs = vec![
            Log::PayFee {
                from: transaction.sender.clone(),
                fee: transaction.fee,
            },
            Log::transfer_log_from_transaction(transaction),
        ];
        Ok(AccountInfo::with_receipt(receipt, logs))
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

        let vesting = match receipt {
            None => {
                let account = match complete!(get_or_update_account::<Account>(
                    accounts_tree,
                    db_txn,
                    &key
                )) {
                    Some(account) => account,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: transaction.sender.clone(),
                        });
                    }
                };

                if let Account::Vesting(contract) = account {
                    contract
                } else {
                    return Err(AccountError::TypeMismatch {
                        expected: AccountType::Vesting,
                        got: account.account_type(),
                    });
                }
            }
            Some(r) => {
                let receipt: VestingReceipt = Deserialize::deserialize_from_vec(r)?;

                VestingContract::from(receipt)
            }
        };

        let new_balance = Account::balance_add(vesting.balance, transaction.total_value())?;

        accounts_tree
            .put(
                db_txn,
                &key,
                Account::Vesting(vesting.change_balance(new_balance)),
            )
            .expect("temporary until accounts rewrite");

        Ok(vec![
            Log::PayFee {
                from: transaction.sender.clone(),
                fee: transaction.fee,
            },
            Log::transfer_log_from_transaction(transaction),
        ])
    }
    fn commit_failed_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
    ) -> Result<AccountInfo, AccountError> {
        let key = KeyNibbles::from(&transaction.sender);

        let account = match complete!(get_or_update_account::<Account>(
            accounts_tree,
            db_txn,
            &key
        )) {
            Some(account) => account,
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: transaction.sender.clone(),
                });
            }
        };

        let vesting = match account {
            Account::Vesting(ref value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Vesting,
                    got: account.account_type(),
                })
            }
        };

        // Note that in this type of transactions the fee is paid (deducted) from the contract balance
        let new_balance = Account::balance_sub(account.balance(), transaction.fee)?;

        // Store the account or prune if necessary.
        let receipt = if new_balance.is_zero() {
            accounts_tree.remove(db_txn, &key);

            Some(VestingReceipt::from(vesting.clone()).serialize_to_vec())
        } else {
            accounts_tree
                .put(
                    db_txn,
                    &key,
                    Account::Vesting(vesting.change_balance(new_balance)),
                )
                .expect("temporary until accounts rewrite");

            None
        };
        let logs = vec![Log::PayFee {
            from: transaction.sender.clone(),
            fee: transaction.fee,
        }];
        Ok(AccountInfo::with_receipt(receipt, logs))
    }
    fn revert_failed_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        receipt: Option<&Vec<u8>>,
    ) -> Result<Vec<Log>, AccountError> {
        let key = KeyNibbles::from(&transaction.sender);

        let vesting = match receipt {
            None => {
                let account = match complete!(get_or_update_account::<Account>(
                    accounts_tree,
                    db_txn,
                    &key
                )) {
                    Some(account) => account,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: transaction.sender.clone(),
                        });
                    }
                };

                if let Account::Vesting(contract) = account {
                    contract
                } else {
                    return Err(AccountError::TypeMismatch {
                        expected: AccountType::Vesting,
                        got: account.account_type(),
                    });
                }
            }
            Some(r) => {
                let receipt: VestingReceipt = Deserialize::deserialize_from_vec(r)?;

                VestingContract::from(receipt)
            }
        };

        let new_balance = Account::balance_add(vesting.balance, transaction.fee)?;

        accounts_tree
            .put(
                db_txn,
                &key,
                Account::Vesting(vesting.change_balance(new_balance)),
            )
            .expect("temporary until accounts rewrite");

        Ok(vec![Log::PayFee {
            from: transaction.sender.clone(),
            fee: transaction.fee,
        }])
    }

    fn can_pay_fee(
        &self,
        transaction: &Transaction,
        mempool_balance: Coin,
        block_time: u64,
    ) -> bool {
        let new_balance = match Account::balance_sub(self.balance, mempool_balance) {
            Ok(new_balance) => new_balance,
            Err(_) => return false,
        };

        // Check vesting min cap.
        let min_cap = self.min_cap(block_time);

        if new_balance < min_cap {
            return false;
        }

        // Check transaction signer is contract owner.
        let signature_proof: SignatureProof =
            match Deserialize::deserialize(&mut &transaction.proof[..]) {
                Ok(proof) => proof,
                Err(_) => return false,
            };

        if !signature_proof.is_signed_by(&self.owner) {
            return false;
        }

        true
    }

    fn delete(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
    ) -> Result<Vec<Log>, AccountError> {
        let key = KeyNibbles::from(&transaction.contract_creation_address());

        let account = match complete!(get_or_update_account::<Account>(
            accounts_tree,
            db_txn,
            &key
        )) {
            Some(account) => account,
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: transaction.sender.clone(),
                });
            }
        };

        let vesting = match account {
            Account::Vesting(ref value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Vesting,
                    got: account.account_type(),
                })
            }
        };

        let previous_balance = Account::balance_sub(vesting.balance, transaction.value)?;

        if previous_balance == Coin::ZERO {
            // If the previous balance was zero, we just remove the account from the accounts tree
            accounts_tree.remove(db_txn, &key);
        } else {
            // If the previous balance was not zero, we need to restore the basic account with the previous balance
            accounts_tree
                .put(
                    db_txn,
                    &key,
                    Account::Basic(BasicAccount {
                        balance: previous_balance,
                    }),
                )
                .expect("temporary until accounts rewrite");
        }
        Ok(Vec::new())
    }
}

impl AccountInherentInteraction for VestingContract {
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
pub struct VestingReceipt {
    pub owner: Address,
    pub start_time: u64,
    pub time_step: u64,
    pub step_amount: Coin,
    pub total_amount: Coin,
}

impl From<VestingContract> for VestingReceipt {
    fn from(contract: VestingContract) -> Self {
        VestingReceipt {
            owner: contract.owner,
            start_time: contract.start_time,
            time_step: contract.time_step,
            step_amount: contract.step_amount,
            total_amount: contract.total_amount,
        }
    }
}

impl From<VestingReceipt> for VestingContract {
    fn from(receipt: VestingReceipt) -> Self {
        VestingContract {
            balance: Coin::ZERO,
            owner: receipt.owner,
            start_time: receipt.start_time,
            time_step: receipt.time_step,
            step_amount: receipt.step_amount,
            total_amount: receipt.total_amount,
        }
    }
}
