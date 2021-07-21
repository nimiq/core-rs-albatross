use log::error;

use beserial::{Deserialize, Serialize};
use nimiq_database::WriteTransaction;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;

use crate::staking_contract::receipts::{DropStakerReceipt, UpdateStakerReceipt};
use crate::staking_contract::RetireStakerReceipt;
use crate::{Account, AccountError, AccountsTree, StakingContract};

/// Struct representing a staker in the staking contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staker {
    // The address of the staker.
    pub address: Address,
    // The portion of the staker's balance that is currently active.
    pub active_stake: Coin,
    // The portion of the staker's balance that is currently inactive.
    pub inactive_stake: Coin,
    // The id of the validator for which the staker is delegating its stake for. If it is not
    // delegating to any validator, this will be set to None.
    pub delegation: Option<Address>,
    // A field stating when the stake was last retired (in block height).
    pub retire_time: u32,
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
/// Retire and Re-activate are self transactions on the staking address.
/// Unstake is a transaction from the staking contract to an arbitrary address.
impl StakingContract {
    /// Adds funds to stake of `address` for validator `validator_key`.
    /// XXX This is public to fill the genesis staking contract
    pub fn create_staker(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        balance: Coin,
        delegation: Option<Address>,
    ) -> Result<(), AccountError> {
        // See if the staker already exists.
        if StakingContract::get_staker(accounts_tree, db_txn, staker_address).is_some() {
            return Err(AccountError::AlreadyExistentAddress {
                address: staker_address.clone(),
            });
        }

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_add(staking_contract.balance, balance)?;

        // Create the staker struct.
        let staker = Staker {
            address: staker_address.clone(),
            active_stake: balance,
            inactive_stake: Coin::ZERO,
            delegation: delegation.clone(),
            retire_time: 0,
        };

        // If we are staking for a validator, we need to update it.
        if let Some(validator_address) = delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, &validator_address) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: validator_address.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_add(validator.balance, balance)?;
            validator.num_stakers += 1;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&validator_address),
                Account::StakingValidator(validator),
            );

            // Add the staker entry to the validator.
            trace!(
                "Trying to put validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(&validator_address, staker_address),
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
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance =
            Account::balance_sub(staking_contract.balance, staker.active_stake)?;

        // If we are staking for a validator, we need to update it.
        if let Some(validator_address) = staker.delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, &validator_address) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: validator_address.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_sub(validator.balance, staker.active_stake)?;
            validator.num_stakers -= 1;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&validator_address),
                Account::StakingValidator(validator),
            );

            // Remove the staker entry from the validator.
            trace!(
                "Trying to remove validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.remove(
                db_txn,
                &StakingContract::get_key_validator_staker(&validator_address, staker_address),
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
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        staker.active_stake = Account::balance_add(staker.active_stake, value)?;

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_add(staking_contract.balance, value)?;

        // If we are staking for a validator, we need to update it too.
        if let Some(validator_address) = &staker.delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: validator_address.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_add(validator.balance, value)?;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_address),
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
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        staker.active_stake = Account::balance_sub(staker.active_stake, value)?;

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_sub(staking_contract.balance, value)?;

        // If we are staking for a validator, we need to update it too.
        if let Some(validator_address) = &staker.delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: validator_address.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_sub(validator.balance, value)?;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_address),
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
        delegation: Option<Address>,
    ) -> Result<UpdateStakerReceipt, AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Do some checks regarding the old and new delegations.
        if let Some(old_validator_address) = &staker.delegation {
            if StakingContract::get_validator(accounts_tree, db_txn, old_validator_address)
                .is_none()
            {
                return Err(AccountError::NonExistentAddress {
                    address: old_validator_address.clone(),
                });
            }
        }

        if let Some(new_validator_address) = &delegation {
            if StakingContract::get_validator(accounts_tree, db_txn, new_validator_address)
                .is_none()
            {
                return Err(AccountError::NonExistentAddress {
                    address: new_validator_address.clone(),
                });
            }
        }

        // All checks passed, not allowed to fail from here on!

        // Create the receipt.
        let receipt = UpdateStakerReceipt {
            old_delegation: staker.delegation.clone(),
        };

        // If we were staking for a validator, we remove ourselves from it.
        if let Some(old_validator_address) = &staker.delegation {
            // Get the validator.
            let mut old_validator =
                StakingContract::get_validator(accounts_tree, db_txn, old_validator_address)
                    .unwrap();

            // Update it.
            old_validator.balance =
                Account::balance_sub(old_validator.balance, staker.active_stake)?;
            old_validator.num_stakers -= 1;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                old_validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(old_validator_address),
                Account::StakingValidator(old_validator),
            );

            // Remove the staker entry from the validator.
            trace!(
                "Trying to remove validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.remove(
                db_txn,
                &StakingContract::get_key_validator_staker(old_validator_address, staker_address),
            );
        }

        // If we are now staking for a validator, we add ourselves to it.
        if let Some(new_validator_address) = &delegation {
            // Get the validator.
            let mut new_validator =
                StakingContract::get_validator(accounts_tree, db_txn, new_validator_address)
                    .unwrap();

            // Update it.
            new_validator.balance =
                Account::balance_add(new_validator.balance, staker.active_stake)?;
            new_validator.num_stakers += 1;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                new_validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(new_validator_address),
                Account::StakingValidator(new_validator),
            );

            // Add the staker entry to the validator.
            trace!(
                "Trying to put validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(new_validator_address, staker_address),
                Account::StakingValidatorsStaker(staker_address.clone()),
            );
        }

        // Update the staker and re-add it to the accounts tree.
        staker.delegation = delegation;

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
        receipt: UpdateStakerReceipt,
    ) -> Result<(), AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Remove ourselves from the current delegation, if it exists.
        if let Some(new_validator_address) = staker.delegation {
            // Get the validator.
            let mut new_validator =
                match StakingContract::get_validator(accounts_tree, db_txn, &new_validator_address)
                {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: new_validator_address.clone(),
                        });
                    }
                };

            // Update it.
            new_validator.balance =
                Account::balance_sub(new_validator.balance, staker.active_stake)?;
            new_validator.num_stakers -= 1;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                new_validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&new_validator_address),
                Account::StakingValidator(new_validator),
            );

            // Remove the staker entry from the validator.
            trace!(
                "Trying to remove validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.remove(
                db_txn,
                &StakingContract::get_key_validator_staker(&new_validator_address, staker_address),
            );
        }

        // Add ourselves to the previous delegation, if it existed.
        if let Some(old_validator_address) = receipt.old_delegation.clone() {
            // Get the validator.
            let mut old_validator =
                match StakingContract::get_validator(accounts_tree, db_txn, &old_validator_address)
                {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: old_validator_address.clone(),
                        });
                    }
                };

            // Update it.
            old_validator.balance =
                Account::balance_add(old_validator.balance, staker.active_stake)?;
            old_validator.num_stakers += 1;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                old_validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&old_validator_address),
                Account::StakingValidator(old_validator),
            );

            // Add the staker entry to the validator.
            trace!(
                "Trying to put validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(&old_validator_address, staker_address),
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
        value: Coin,
        block_height: u32,
    ) -> Result<RetireStakerReceipt, AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Create the receipt.
        let receipt = RetireStakerReceipt {
            old_retire_time: staker.retire_time,
        };

        // Update the staker.
        staker.active_stake = Account::balance_sub(staker.active_stake, value)?;
        staker.inactive_stake = Account::balance_add(staker.inactive_stake, value)?;
        staker.retire_time = block_height;

        // If we are staking for a validator, we update it.
        if let Some(validator_address) = &staker.delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: validator_address.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_sub(validator.balance, value)?;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_address),
                Account::StakingValidator(validator),
            );
        }

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
        value: Coin,
        receipt: RetireStakerReceipt,
    ) -> Result<(), AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Update the staker.
        staker.active_stake = Account::balance_add(staker.active_stake, value)?;
        staker.inactive_stake = Account::balance_sub(staker.inactive_stake, value)?;
        staker.retire_time = receipt.old_retire_time;

        // Update the validator if necessary.
        if let Some(validator_address) = &staker.delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: validator_address.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_add(validator.balance, value)?;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_address),
                Account::StakingValidator(validator),
            );
        }

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
        value: Coin,
    ) -> Result<(), AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Update the staker.
        staker.active_stake = Account::balance_add(staker.active_stake, value)?;
        staker.inactive_stake = Account::balance_sub(staker.inactive_stake, value)?;

        // Update the validator if necessary.
        if let Some(validator_address) = &staker.delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: validator_address.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_add(validator.balance, value)?;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_address),
                Account::StakingValidator(validator),
            );
        }

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

    /// Reverts reactivating a staker.
    pub(crate) fn revert_reactivate_staker(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
    ) -> Result<(), AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Update the staker.
        staker.active_stake = Account::balance_sub(staker.active_stake, value)?;
        staker.inactive_stake = Account::balance_add(staker.inactive_stake, value)?;

        // Update the validator if necessary.
        if let Some(validator_address) = &staker.delegation {
            // Get the validator.
            let mut validator =
                match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                    Some(v) => v,
                    None => {
                        return Err(AccountError::NonExistentAddress {
                            address: validator_address.clone(),
                        });
                    }
                };

            // Update it.
            validator.balance = Account::balance_sub(validator.balance, value)?;

            // Re-add the validator entry.
            trace!(
                "Trying to put validator with address {} in the accounts tree.",
                validator_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_address),
                Account::StakingValidator(validator),
            );
        }

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
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // Check that the staker has been inactive for long enough.
        if block_height < policy::election_block_after(staker.retire_time) {
            error!(
                "Tried to unstake a staker before time! Staker address {}",
                staker_address.clone()
            );

            return Err(AccountError::InvalidForSender);
        }

        // Update the staker.
        staker.inactive_stake = Account::balance_sub(staker.inactive_stake, value)?;

        // All checks passed, not allowed to fail from here on!

        // Re-add or remove the staker entry, depending on remaining balance.
        if staker.active_stake.is_zero() && staker.inactive_stake.is_zero() {
            trace!(
                "Trying to remove staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.remove(db_txn, &StakingContract::get_key_staker(staker_address));

            Ok(Some(DropStakerReceipt {
                delegation: staker.delegation.clone(),
                retire_time: staker.retire_time,
            }))
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
        let mut staker = match receipt {
            Some(x) => Staker {
                address: staker_address.clone(),
                active_stake: Coin::ZERO,
                inactive_stake: Coin::ZERO,
                delegation: x.delegation,
                retire_time: x.retire_time,
            },
            None => StakingContract::get_staker(accounts_tree, db_txn, staker_address).unwrap(),
        };

        staker.inactive_stake = Account::balance_add(staker.inactive_stake, value)?;

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

    /// Used to pay fees
    pub(crate) fn deduct_fees(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        from_active_balance: bool,
        value: Coin,
    ) -> Result<Option<DropStakerReceipt>, AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // See if the fees are to be deducted from the active or the inactive balance.
        if from_active_balance {
            staker.active_stake = Account::balance_sub(staker.active_stake, value)?;
        } else {
            staker.inactive_stake = Account::balance_sub(staker.inactive_stake, value)?;
        }

        // All checks passed, not allowed to fail from here on!

        // Re-add or remove the staker entry, depending on remaining balance.
        if staker.active_stake.is_zero() && staker.inactive_stake.is_zero() {
            trace!(
                "Trying to remove staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.remove(db_txn, &StakingContract::get_key_staker(staker_address));

            Ok(Some(DropStakerReceipt {
                delegation: staker.delegation.clone(),
                retire_time: staker.retire_time,
            }))
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

    /// Reverts a deduct fees transaction.
    pub(crate) fn revert_deduct_fees(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        from_active_balance: bool,
        value: Coin,
        receipt: Option<DropStakerReceipt>,
    ) -> Result<(), AccountError> {
        let mut staker = match receipt {
            Some(x) => Staker {
                address: staker_address.clone(),
                active_stake: Coin::ZERO,
                inactive_stake: Coin::ZERO,
                delegation: x.delegation,
                retire_time: x.retire_time,
            },
            None => StakingContract::get_staker(accounts_tree, db_txn, staker_address).unwrap(),
        };

        if from_active_balance {
            staker.active_stake = Account::balance_add(staker.active_stake, value)?;
        } else {
            staker.inactive_stake = Account::balance_add(staker.inactive_stake, value)?;
        }

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
}
