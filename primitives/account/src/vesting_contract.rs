use beserial::{Deserialize, Serialize};
use nimiq_database::WriteTransaction;
use nimiq_keys::Address;
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::vesting_contract::CreationTransactionData;
use nimiq_transaction::{SignatureProof, Transaction};
use nimiq_trie::key_nibbles::KeyNibbles;

use crate::inherent::Inherent;
use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::{Account, AccountError, AccountsTree};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
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
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        balance: Coin,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<(), AccountError> {
        let data = CreationTransactionData::parse(transaction)?;

        let contract_key = KeyNibbles::from(&transaction.contract_creation_address());

        if accounts_tree.get(db_txn, &contract_key).is_some() {
            return Err(AccountError::AlreadyExistentAddress {
                address: transaction.contract_creation_address(),
            });
        }

        let contract = VestingContract::new(
            balance,
            data.owner,
            data.start_time,
            data.time_step,
            data.step_amount,
            data.total_amount,
        );

        accounts_tree.put(db_txn, &contract_key, Account::Vesting(contract));

        Ok(())
    }

    fn commit_incoming_transaction(
        _accounts_tree: &AccountsTree,
        _db_txn: &mut WriteTransaction,
        _transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_incoming_transaction(
        _accounts_tree: &AccountsTree,
        _db_txn: &mut WriteTransaction,
        _transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        _receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_outgoing_transaction(
        accounts_tree: &AccountsTree,
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

        let vesting = match account {
            Account::Vesting(ref value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Vesting,
                    got: account.account_type(),
                })
            }
        };

        let new_balance = Account::balance_sub(account.balance(), transaction.total_value()?)?;

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

        accounts_tree.put(
            db_txn,
            &key,
            Account::Vesting(vesting.change_balance(new_balance)),
        );

        Ok(None)
    }

    fn revert_outgoing_transaction(
        accounts_tree: &AccountsTree,
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

        let vesting = match account {
            Account::Vesting(ref value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Vesting,
                    got: account.account_type(),
                })
            }
        };

        let new_balance = Account::balance_add(account.balance(), transaction.total_value()?)?;

        accounts_tree.put(
            db_txn,
            &key,
            Account::Vesting(vesting.change_balance(new_balance)),
        );

        Ok(())
    }
}

impl AccountInherentInteraction for VestingContract {
    fn commit_inherent(
        _accounts_tree: &AccountsTree,
        _db_txn: &mut WriteTransaction,
        _inherent: &Inherent,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        Err(AccountError::InvalidInherent)
    }

    fn revert_inherent(
        _accounts_tree: &AccountsTree,
        _db_txn: &mut WriteTransaction,
        _inherent: &Inherent,
        _block_height: u32,
        _block_time: u64,
        _receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidInherent)
    }
}
