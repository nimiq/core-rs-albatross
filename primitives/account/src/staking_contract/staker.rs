use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use log::error;
use parking_lot::RwLock;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_database::WriteTransaction;
use nimiq_keys::Address;
use nimiq_primitives::account::ValidatorId;
use nimiq_primitives::coin::Coin;

use crate::staking_contract::receipts::{
    DropStakerReceipt, ReactivateValidatorOrStakerReceipt, UpdateOrRetireStakerReceipt,
};
use crate::{Account, AccountError, AccountsTree, StakingContract};
use nimiq_primitives::policy;

/// Struct representing a staker in the staking contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staker {
    // The reward address of the staker.
    pub address: Address,
    // The balance of the stake.
    pub balance: Coin,
    // The id of the validator for which it is staking for. If it is not staking for any validator,
    // this will be set to None.
    pub delegation: Option<ValidatorId>,
    // A flag stating if the staker is inactive. If it is inactive, then it contains the block
    // height at which it became inactive.
    pub inactivity_flag: Option<u32>,
}

/// Actions concerning a staker are:
/// 1. Stake: Delegate stake from an outside address to a validator.
/// 2. Retire: Remove stake from a validator and make it inactive
///            (starting the cooldown period for Unstake).
/// 3. Re-activate: Re-delegate inactive stake to a validator.
/// 4. Unstake: Remove inactive stake from the staking contract
///             (after it has been inactive for the cooldown period).
///
/// The actions can be summarized by the following state diagram:
///        +--------+   retire    +----------+
/// stake  |        +------------>+          | unstake
///+------>+ staked |             | inactive +--------->
///        |        +<------------+          |
///        +--------+ re-activate +----------+
///
/// Stake is a transaction from an arbitrary address to the staking contract.
/// Retire and Re-activate are self transactions on the staking address. TODO: Change this.
/// Unstake is a transaction from the staking contract to an arbitrary address.
impl StakingContract {
    /// Adds funds to stake of `address` for validator `validator_key`.
    /// XXX This is public to fill the genesis staking contract
    pub fn create_staker(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        balance: Coin,
        delegation: Option<ValidatorId>,
    ) -> Result<(), AccountError> {
        // See if the staker already exists.
        if StakingContract::get_staker(accounts_tree, db_txn, staker_address).is_some() {
            return Err(AccountError::AlreadyExistentStaker {
                address: staker_address.clone(),
            });
        }

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_add(staking_contract.balance, balance)?;

        // Create the staker struct.
        let staker = Staker {
            address: staker_address.clone(),
            balance,
            delegation: delegation.clone(),
            inactivity_flag: None,
        };

        // If we are staking for a validator, we need to update it.
        if let Some(validator_id) = delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, &validator_id) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentValidator {
                            id: validator_id.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_add(validator.balance, balance)?;
            validator.num_stakers += 1;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with id {} in the accounts tree.",
                validator_id.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&validator_id),
                Account::StakingValidator(validator),
            );

            // Add the staker entry to the validator.
            trace!(
                "Trying to put validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(&validator_id, &staker_address),
                Account::StakingValidatorsStaker(staker_address.clone()),
            );
        }

        // Add the staking contract and the staker entries.
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put staker with address {} in the accounts tree.",
            staker_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(())
    }

    /// Reverts a stake transaction.
    pub(crate) fn revert_create_staker(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
    ) -> Result<(), AccountError> {
        // Get the staker and check if it exists.
        let staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_sub(staking_contract.balance, staker.balance)?;

        // If we are staking for a validator, we need to update it.
        if let Some(validator_id) = staker.delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, &validator_id) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentValidator {
                            id: validator_id.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_sub(validator.balance, staker.balance)?;
            validator.num_stakers -= 1;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with id {} in the accounts tree.",
                validator_id.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&validator_id),
                Account::StakingValidator(validator),
            );

            // Remove the staker entry from the validator.
            trace!(
                "Trying to remove validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.remove(
                db_txn,
                &StakingContract::get_key_validator_staker(&validator_id, &staker_address),
            );
        }

        // Add the staking contract entry.
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        // Remove the staker entry.
        trace!(
            "Trying to remove staker with address {} in the accounts tree.",
            staker_address.to_string(),
        );

        accounts_tree.remove(db_txn, &StakingContract::get_key_staker(staker_address));

        Ok(())
    }

    pub(crate) fn stake(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
    ) -> Result<(), AccountError> {
        // Get the staker, check if it exists and update it.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        staker.balance = Account::balance_add(staker.balance, value)?;

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_add(staking_contract.balance, value)?;

        // If we are staking for a validator, we need to update it too.
        if let Some(validator_id) = &staker.delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, validator_id) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentValidator {
                            id: validator_id.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_add(validator.balance, value)?;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with id {} in the accounts tree.",
                validator_id.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_id),
                Account::StakingValidator(validator),
            );
        }

        // Add the staking contract and the staker entries.
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put staker with address {} in the accounts tree.",
            staker_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(())
    }

    pub(crate) fn revert_stake(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
    ) -> Result<(), AccountError> {
        // Get the staker, check if it exists and update it.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        staker.balance = Account::balance_sub(staker.balance, value)?;

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_sub(staking_contract.balance, value)?;

        // If we are staking for a validator, we need to update it too.
        if let Some(validator_id) = &staker.delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, validator_id) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentValidator {
                            id: validator_id.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_sub(validator.balance, value)?;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with id {} in the accounts tree.",
                validator_id.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_id),
                Account::StakingValidator(validator),
            );
        }

        // Add the staking contract and the staker entries.
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put staker with address {} in the accounts tree.",
            staker_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(())
    }

    /// Update staker details.
    pub(crate) fn update_staker(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        delegation: Option<ValidatorId>,
    ) -> Result<UpdateOrRetireStakerReceipt, AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Do some checks regarding the old and new delegations.
        if let Some(old_validator_id) = &staker.delegation {
            if StakingContract::get_validator(accounts_tree, db_txn, old_validator_id).is_none() {
                return Err(AccountError::NonExistentValidator {
                    id: old_validator_id.clone(),
                });
            }
        }

        if let Some(new_validator_id) = &delegation {
            if StakingContract::get_validator(accounts_tree, db_txn, new_validator_id).is_none() {
                return Err(AccountError::NonExistentValidator {
                    id: new_validator_id.clone(),
                });
            }
        }

        // All checks passed, not allowed to fail from here on!

        // Create the receipt.
        let receipt = UpdateOrRetireStakerReceipt {
            old_delegation: staker.delegation.clone(),
        };

        // If we were staking for a validator, we remove ourselves from it.
        if let Some(old_validator_id) = &staker.delegation {
            // Get the validator.
            let mut old_validator =
                StakingContract::get_validator(accounts_tree, db_txn, old_validator_id).unwrap();

            // Update it.
            old_validator.balance = Account::balance_sub(old_validator.balance, staker.balance)?;
            old_validator.num_stakers -= 1;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with id {} in the accounts tree.",
                old_validator_id.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&old_validator_id),
                Account::StakingValidator(old_validator),
            );

            // Remove the staker entry from the validator.
            trace!(
                "Trying to remove validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.remove(
                db_txn,
                &StakingContract::get_key_validator_staker(&old_validator_id, &staker_address),
            );
        }

        // If we are now staking for a validator, we add ourselves to it.
        if let Some(new_validator_id) = &delegation {
            // Get the validator.
            let mut new_validator =
                StakingContract::get_validator(accounts_tree, db_txn, new_validator_id).unwrap();

            // Update it.
            new_validator.balance = Account::balance_add(new_validator.balance, staker.balance)?;
            new_validator.num_stakers += 1;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with id {} in the accounts tree.",
                new_validator_id.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&new_validator_id),
                Account::StakingValidator(new_validator),
            );

            // Add the staker entry to the validator.
            trace!(
                "Trying to put validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(&new_validator_id, &staker_address),
                Account::StakingValidatorsStaker(staker_address.clone()),
            );
        }

        // Update the staker and re-add it to the accounts tree.
        staker.delegation = delegation.clone();

        trace!(
            "Trying to put staker with address {} in the accounts tree.",
            staker_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(receipt)
    }

    /// Reverts updating staker details.
    pub(crate) fn revert_update_staker(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        receipt: UpdateOrRetireStakerReceipt,
    ) -> Result<(), AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Remove ourselves from the current delegation, if it exists.
        if let Some(new_validator_id) = staker.delegation {
            // Get the validator.
            let mut new_validator =
                match StakingContract::get_validator(accounts_tree, db_txn, &new_validator_id) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentValidator {
                            id: new_validator_id.clone(),
                        });
                    }
                };

            // Update it.
            new_validator.balance = Account::balance_sub(new_validator.balance, staker.balance)?;
            new_validator.num_stakers -= 1;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with id {} in the accounts tree.",
                new_validator_id.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&new_validator_id),
                Account::StakingValidator(new_validator),
            );

            // Remove the staker entry from the validator.
            trace!(
                "Trying to remove validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.remove(
                db_txn,
                &StakingContract::get_key_validator_staker(&new_validator_id, &staker_address),
            );
        }

        // Add ourselves to the previous delegation, if it existed.
        if let Some(old_validator_id) = receipt.old_delegation.clone() {
            // Get the validator.
            let mut old_validator =
                match StakingContract::get_validator(accounts_tree, db_txn, &old_validator_id) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentValidator {
                            id: old_validator_id.clone(),
                        });
                    }
                };

            // Update it.
            old_validator.balance = Account::balance_add(old_validator.balance, staker.balance)?;
            old_validator.num_stakers += 1;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with id {} in the accounts tree.",
                old_validator_id.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&old_validator_id),
                Account::StakingValidator(old_validator),
            );

            // Add the staker entry to the validator.
            trace!(
                "Trying to put validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(&old_validator_id, &staker_address),
                Account::StakingValidatorsStaker(staker_address.clone()),
            );
        }

        // Update the staker and re-add it to the accounts tree.
        staker.delegation = receipt.old_delegation;

        trace!(
            "Trying to put staker with address {} in the accounts tree.",
            staker_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(())
    }

    /// Retires a staker.
    pub(crate) fn retire_staker(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        block_height: u32,
    ) -> Result<UpdateOrRetireStakerReceipt, AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Check if the staker is not already inactive.
        if staker.inactivity_flag.is_some() {
            error!(
                "Tried to inactivate a staker that was already inactivated! It has address {}.",
                staker_address
            );
            return Err(AccountError::InvalidForRecipient);
        }

        // Get the current delegation and create the receipt.
        let old_delegation = staker.delegation;

        let receipt = UpdateOrRetireStakerReceipt {
            old_delegation: old_delegation.clone(),
        };

        // If we were staking for a validator, we remove ourselves from it.
        if let Some(old_validator_id) = old_delegation {
            // Get the validator.
            let mut old_validator =
                match StakingContract::get_validator(accounts_tree, db_txn, &old_validator_id) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentValidator {
                            id: old_validator_id.clone(),
                        });
                    }
                };

            // Update it.
            old_validator.balance = Account::balance_sub(old_validator.balance, staker.balance)?;
            old_validator.num_stakers -= 1;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with id {} in the accounts tree.",
                old_validator_id.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&old_validator_id),
                Account::StakingValidator(old_validator),
            );

            // Remove the staker entry from the validator.
            trace!(
                "Trying to remove validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.remove(
                db_txn,
                &StakingContract::get_key_validator_staker(&old_validator_id, &staker_address),
            );
        }

        // Update the staker.
        staker.delegation = None;
        staker.inactivity_flag = Some(block_height);

        // Re-add the staker entry.
        trace!(
            "Trying to put staker with address {} in the accounts tree.",
            staker_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(receipt)
    }

    /// Reverts retiring a staker.
    pub(crate) fn revert_retire_staker(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        receipt: UpdateOrRetireStakerReceipt,
    ) -> Result<(), AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Add ourselves to the previous delegation, if it existed.
        if let Some(old_validator_id) = receipt.old_delegation.clone() {
            // Get the validator.
            let mut old_validator =
                match StakingContract::get_validator(accounts_tree, db_txn, &old_validator_id) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentValidator {
                            id: old_validator_id.clone(),
                        });
                    }
                };

            // Update it.
            old_validator.balance = Account::balance_add(old_validator.balance, staker.balance)?;
            old_validator.num_stakers += 1;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with id {} in the accounts tree.",
                old_validator_id.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&old_validator_id),
                Account::StakingValidator(old_validator),
            );

            // Add the staker entry to the validator.
            trace!(
                "Trying to put validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(&old_validator_id, &staker_address),
                Account::StakingValidatorsStaker(staker_address.clone()),
            );
        }

        // Update the staker and re-add it to the accounts tree.
        staker.delegation = receipt.old_delegation;
        staker.inactivity_flag = None;

        trace!(
            "Trying to put staker with address {} in the accounts tree.",
            staker_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(())
    }

    /// Reactivates a staker.
    pub(crate) fn reactivate_staker(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
    ) -> Result<ReactivateValidatorOrStakerReceipt, AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Create the receipt.
        let receipt = match staker.inactivity_flag {
            None => {
                error!(
                    "Tried to reactivate a staker that was already active! It has address {}.",
                    staker_address
                );
                return Err(AccountError::InvalidForRecipient);
            }
            Some(retire_time) => ReactivateValidatorOrStakerReceipt { retire_time },
        };

        // Update the staker.
        staker.inactivity_flag = None;

        // Re-add the staker entry.
        trace!(
            "Trying to put staker with address {} in the accounts tree.",
            staker_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(receipt)
    }

    /// Reverts reactivating a staker.
    pub(crate) fn revert_reactivate_staker(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        receipt: ReactivateValidatorOrStakerReceipt,
    ) -> Result<(), AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Update the staker and re-add it to the accounts tree.
        staker.inactivity_flag = Some(receipt.retire_time);

        trace!(
            "Trying to put staker with address {} in the accounts tree.",
            staker_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(())
    }

    /// Removes stake from an inactive staker. If the entire stake is removed then the staker is dropped.
    pub(crate) fn unstake(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
        block_height: u32,
    ) -> Result<Option<DropStakerReceipt>, AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Check that the staker has been inactive for long enough.
        match staker.inactivity_flag {
            None => {
                error!(
                    "Tried to drop a staker which was still active! Staker address {}",
                    staker_address.clone()
                );
                return Err(AccountError::InvalidForSender);
            }
            Some(time) => {
                if block_height < policy::election_block_after(time) {
                    return Err(AccountError::InvalidForSender);
                }
            }
        }

        // Create the receipt.
        let receipt = match staker.inactivity_flag {
            None => {
                error!(
                    "Tried to unstaker from an active staker! It has address {}.",
                    staker_address
                );
                return Err(AccountError::InvalidForRecipient);
            }
            Some(retire_time) => DropStakerReceipt { retire_time },
        };

        // Update the staker.
        staker.balance = Account::balance_sub(staker.balance, value)?;

        // All checks passed, not allowed to fail from here on!

        // Re-add or remove the staker entry, depending on remaining balance.
        if staker.balance.is_zero() {
            trace!(
                "Trying to remove staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.remove(db_txn, &StakingContract::get_key_staker(staker_address));

            Ok(Some(receipt))
        } else {
            trace!(
                "Trying to put staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_staker(staker_address),
                Account::StakingStaker(staker),
            );

            Ok(None)
        }
    }

    /// Reverts a unstake transaction.
    pub(crate) fn revert_unstake(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
        receipt: Option<DropStakerReceipt>,
    ) -> Result<(), AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentStaker {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Update the staker.
        staker.balance = Account::balance_sub(staker.balance, value)?;

        // Check if the staker was dropped or not.
        if receipt.is_some() {
            // If yes, re-create the staker and add it to the accounts tree.
            let staker = Staker {
                address: staker_address.clone(),
                balance: value,
                delegation: None,
                inactivity_flag: Some(receipt.unwrap().retire_time),
            };

            trace!(
                "Trying to put staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_staker(staker_address),
                Account::StakingStaker(staker),
            );
        } else {
            // If not, get the staker and update it.
            let mut staker =
                StakingContract::get_staker(accounts_tree, db_txn, staker_address).unwrap();

            staker.balance = Account::balance_add(staker.balance, value)?;

            trace!(
                "Trying to put staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_staker(staker_address),
                Account::StakingStaker(staker),
            );
        }

        Ok(())
    }
}
