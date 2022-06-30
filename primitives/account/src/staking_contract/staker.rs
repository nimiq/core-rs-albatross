use beserial::{Deserialize, Serialize};
use nimiq_database::WriteTransaction;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;

use crate::logs::{Log, OperationInfo};
use crate::staking_contract::receipts::StakerReceipt;
use crate::{Account, AccountError, AccountsTrie, Receipt, StakingContract};

/// Struct representing a staker in the staking contract.
/// Actions concerning a staker are:
/// 1. Create: Creates a staker.
/// 2. Stake: Adds coins from any outside address to a staker's balance.
/// 3. Update: Updates the validator.
/// 4. Unstake: Removes coins from a staker's balance to outside the staking contract.
///
/// Create, Stake and Update are incoming transactions to the staking contract.
/// Unstake is an outgoing transaction from the staking contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staker {
    // The address of the staker. The corresponding key is used for all transactions (except Stake
    // which is open to any address).
    pub address: Address,
    // The staker's balance.
    pub balance: Coin,
    // The address of the validator for which the staker is delegating its stake for. If it is not
    // delegating to any validator, this will be set to None.
    pub delegation: Option<Address>,
}

impl StakingContract {
    /// Creates a new staker. This function is public to fill the genesis staking contract.
    /// The OperationInfo has always receipt = None, thus the type instationtion of the generic type to Receipt is irrelevant.
    pub fn create_staker(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
        delegation: Option<Address>,
    ) -> Result<OperationInfo<Receipt>, AccountError> {
        // See if the staker already exists.
        if StakingContract::get_staker(accounts_tree, db_txn, staker_address).is_some() {
            return Err(AccountError::AlreadyExistentAddress {
                address: staker_address.clone(),
            });
        }

        // Get the staking contract and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_add(staking_contract.balance, value)?;

        // Create the staker struct.
        let staker = Staker {
            address: staker_address.clone(),
            balance: value,
            delegation: delegation.clone(),
        };

        // Build the return logs
        let logs = vec![Log::CreateStaker {
            staker_address: staker_address.clone(),
            validator_address: delegation.clone(),
            value,
        }];

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
            validator.balance = Account::balance_add(validator.balance, value)?;

            if validator.inactivity_flag.is_none() {
                staking_contract
                    .active_validators
                    .insert(validator_address.clone(), validator.balance);
            }

            validator.num_stakers += 1;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&validator_address),
                Account::StakingValidator(validator),
            );

            // Add the staker entry to the validator.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(&validator_address, staker_address),
                Account::StakingValidatorsStaker(staker_address.clone()),
            );
        }

        // Add the staking contract and the staker entries.
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(OperationInfo {
            receipt: None,
            logs,
        })
    }

    /// Reverts a create staker transaction.
    pub(crate) fn revert_create_staker(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
    ) -> Result<Vec<Log>, AccountError> {
        // Get the staker and check if it exists.
        let staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        // If the transaction value is less than the staker's balance, this means that the original
        // `create_staker` transaction failed and got downgraded to a `stake` transaction.
        // In this case we simply revert the `stake` transaction.
        debug_assert!(value <= staker.balance);
        if value < staker.balance {
            return StakingContract::revert_stake(accounts_tree, db_txn, staker_address, value);
        }

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_sub(staking_contract.balance, staker.balance)?;

        // If we are staking for a validator, we need to update it.
        if let Some(validator_address) = staker.delegation.clone() {
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
            validator.balance = Account::balance_sub(validator.balance, staker.balance)?;

            if validator.inactivity_flag.is_none() {
                staking_contract
                    .active_validators
                    .insert(validator_address.clone(), validator.balance);
            }

            validator.num_stakers -= 1;

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&validator_address),
                Account::StakingValidator(validator),
            );

            // Remove the staker entry from the validator.
            accounts_tree.remove(
                db_txn,
                &StakingContract::get_key_validator_staker(&validator_address, staker_address),
            );
        }

        // Add the staking contract entry.
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        // Remove the staker entry.
        accounts_tree.remove(db_txn, &StakingContract::get_key_staker(staker_address));

        Ok(vec![Log::CreateStaker {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation,
            value,
        }])
    }

    /// Adds stake to a staker. It will be directly added to the staker's balance. Anyone can
    /// stake for a staker.
    /// If a staker at the address doesn't exist, one will be created.
    /// The OperationInfo has always receipt = None, thus the type instationtion of the generic type to Receipt is irrelevant.
    pub(crate) fn stake(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
    ) -> Result<OperationInfo<Receipt>, AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                error!("Couldn't find the staker to which a stake transaction was destined. Plan B: Create a new staker at this address!");

                Staker {
                    address: staker_address.clone(),
                    balance: Coin::ZERO,
                    delegation: None,
                }
            }
            Some(x) => x,
        };

        // Update the balance.
        staker.balance = Account::balance_add(staker.balance, value)?;

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_add(staking_contract.balance, value)?;

        // Build the return logs
        let logs = vec![Log::Stake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
        }];

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

            if validator.inactivity_flag.is_none() {
                staking_contract
                    .active_validators
                    .insert(validator_address.clone(), validator.balance);
            }

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_address),
                Account::StakingValidator(validator),
            );
        }

        // Add the staking contract and the staker entries.
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        Ok(OperationInfo {
            receipt: None,
            logs,
        })
    }

    /// Reverts a stake transaction.
    pub(crate) fn revert_stake(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
    ) -> Result<Vec<Log>, AccountError> {
        // Get the staker, check if it exists and update it.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentAddress {
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

            if validator.inactivity_flag.is_none() {
                staking_contract
                    .active_validators
                    .insert(validator_address.clone(), validator.balance);
            }

            // All checks passed, not allowed to fail from here on!

            // Re-add the validator entry.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_address),
                Account::StakingValidator(validator),
            );
        }

        // Add the staking contract entries.
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        // Add or remove the staker entry, depending on remaining balance.
        if staker.balance.is_zero() {
            accounts_tree.remove(db_txn, &StakingContract::get_key_staker(staker_address));
        } else {
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_staker(staker_address),
                Account::StakingStaker(staker.clone()),
            );
        }

        Ok(vec![Log::Stake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation,
            value,
        }])
    }

    /// Updates the staker details. Right now you can only update the delegation.
    pub(crate) fn update_staker(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        delegation: Option<Address>,
    ) -> Result<OperationInfo<StakerReceipt>, AccountError> {
        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                error!("Tried to update a staker that doesn't exist!");

                return Ok(OperationInfo::with_receipt(
                    StakerReceipt {
                        no_op: true,
                        delegation: None,
                    },
                    vec![],
                ));
            }
            Some(x) => x,
        };

        // Get the staking contract main.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        // Check that the validator from the new delegation exists.
        if let Some(new_validator_address) = &delegation {
            if StakingContract::get_validator(accounts_tree, db_txn, new_validator_address)
                .is_none()
            {
                error!("Tried to delegate to a validator that doesn't exist!");

                return Ok(OperationInfo::with_receipt(
                    StakerReceipt {
                        no_op: true,
                        delegation: None,
                    },
                    vec![],
                ));
            }
        }

        // All checks passed, not allowed to fail from here on!

        // Create the receipt and logs.
        let receipt = StakerReceipt {
            no_op: false,
            delegation: staker.delegation.clone(),
        };

        let logs = vec![Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: staker.delegation.clone(),
            new_validator_address: delegation.clone(),
        }];

        // If we were staking for a validator, we remove ourselves from it.
        if let Some(old_validator_address) = &staker.delegation {
            // Get the validator.
            let mut old_validator =
                StakingContract::get_validator(accounts_tree, db_txn, old_validator_address)
                    .unwrap();

            // Update it.
            old_validator.balance = Account::balance_sub(old_validator.balance, staker.balance)?;

            if old_validator.inactivity_flag.is_none() {
                staking_contract
                    .active_validators
                    .insert(old_validator_address.clone(), old_validator.balance);
            }

            old_validator.num_stakers -= 1;

            // Re-add the validator entry.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(old_validator_address),
                Account::StakingValidator(old_validator),
            );

            // Remove the staker entry from the validator.
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
            new_validator.balance = Account::balance_add(new_validator.balance, staker.balance)?;

            if new_validator.inactivity_flag.is_none() {
                staking_contract
                    .active_validators
                    .insert(new_validator_address.clone(), new_validator.balance);
            }

            new_validator.num_stakers += 1;

            // Re-add the validator entry.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(new_validator_address),
                Account::StakingValidator(new_validator),
            );

            // Add the staker entry to the validator.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(new_validator_address, staker_address),
                Account::StakingValidatorsStaker(staker_address.clone()),
            );
        }

        // Update the staker and re-add it to the accounts tree.
        staker.delegation = delegation;

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        // Save the staking contract.
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );
        Ok(OperationInfo::with_receipt(receipt, logs))
    }

    /// Reverts updating staker details.
    pub(crate) fn revert_update_staker(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        receipt: StakerReceipt,
    ) -> Result<Vec<Log>, AccountError> {
        // If it was a no-op, we end right here.
        if receipt.no_op {
            return Ok(vec![]);
        }

        // Get the staking contract main.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        // Get the staker and check if it exists.
        let mut staker = match StakingContract::get_staker(accounts_tree, db_txn, staker_address) {
            None => {
                return Err(AccountError::NonExistentAddress {
                    address: staker_address.clone(),
                });
            }
            Some(x) => x,
        };

        let log = Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: receipt.delegation.clone(),
            new_validator_address: staker.delegation.clone(),
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
            new_validator.balance = Account::balance_sub(new_validator.balance, staker.balance)?;

            if new_validator.inactivity_flag.is_none() {
                staking_contract
                    .active_validators
                    .insert(new_validator_address.clone(), new_validator.balance);
            }

            new_validator.num_stakers -= 1;

            // Re-add the validator entry.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&new_validator_address),
                Account::StakingValidator(new_validator),
            );

            // Remove the staker entry from the validator.
            accounts_tree.remove(
                db_txn,
                &StakingContract::get_key_validator_staker(&new_validator_address, staker_address),
            );
        }

        // Add ourselves to the previous delegation, if it existed.
        if let Some(old_validator_address) = receipt.delegation.clone() {
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
            old_validator.balance = Account::balance_add(old_validator.balance, staker.balance)?;

            if old_validator.inactivity_flag.is_none() {
                staking_contract
                    .active_validators
                    .insert(old_validator_address.clone(), old_validator.balance);
            }

            old_validator.num_stakers += 1;

            // Re-add the validator entry.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(&old_validator_address),
                Account::StakingValidator(old_validator),
            );

            // Add the staker entry to the validator.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(&old_validator_address, staker_address),
                Account::StakingValidatorsStaker(staker_address.clone()),
            );
        }

        // Update the staker and re-add it to the accounts tree.
        staker.delegation = receipt.delegation;

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker),
        );

        // Save the staking contract.
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(vec![log])
    }

    /// Removes coins from a staker's balance. If the entire staker's balance is unstaked then the
    /// staker is deleted.
    /// The OperationInfo has always receipt = None, thus the type instationtion of the generic type to Receipt is irrelevant.
    pub(crate) fn unstake(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
    ) -> Result<OperationInfo<StakerReceipt>, AccountError> {
        // Get the staking contract.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

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
        staker.balance = Account::balance_sub(staker.balance, value)?;

        // All checks passed, not allowed to fail from here on!

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

            if validator.inactivity_flag.is_none() {
                staking_contract
                    .active_validators
                    .insert(validator_address.clone(), validator.balance);
            }

            // If the staker balance is depleted, we have some extra updates for the validator.
            if staker.balance.is_zero() {
                validator.num_stakers -= 1;

                // Remove the staker address from the validator.
                accounts_tree.remove(
                    db_txn,
                    &StakingContract::get_key_validator_staker(validator_address, &staker.address),
                );
            }

            // Re-add the validator entry.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator(validator_address),
                Account::StakingValidator(validator),
            );
        }

        // Update the staking contract.
        staking_contract.balance = Account::balance_sub(staking_contract.balance, value)?;

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        // Re-add or remove the staker entry, depending on remaining balance.
        if staker.balance.is_zero() {
            accounts_tree.remove(db_txn, &StakingContract::get_key_staker(&staker.address));

            Ok(OperationInfo {
                receipt: Some(StakerReceipt {
                    no_op: false,
                    delegation: staker.delegation.clone(),
                }),
                logs: vec![Log::Unstake {
                    staker_address: staker_address.clone(),
                    validator_address: staker.delegation,
                    value,
                }],
            })
        } else {
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_staker(staker_address),
                Account::StakingStaker(staker.clone()),
            );

            Ok(OperationInfo {
                receipt: None,
                logs: vec![Log::Unstake {
                    staker_address: staker_address.clone(),
                    validator_address: staker.delegation,
                    value,
                }],
            })
        }
    }

    /// Reverts a unstake transaction.
    pub(crate) fn revert_unstake(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        staker_address: &Address,
        value: Coin,
        receipt_opt: Option<StakerReceipt>,
    ) -> Result<Vec<Log>, AccountError> {
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let staker = match receipt_opt {
            Some(receipt) => {
                if let Some(validator_address) = &receipt.delegation {
                    let mut validator = match StakingContract::get_validator(
                        accounts_tree,
                        db_txn,
                        validator_address,
                    ) {
                        Some(v) => v,
                        None => {
                            return Err(AccountError::NonExistentAddress {
                                address: validator_address.clone(),
                            });
                        }
                    };

                    validator.balance = Account::balance_add(validator.balance, value)?;
                    validator.num_stakers += 1;

                    if validator.inactivity_flag.is_none() {
                        staking_contract
                            .active_validators
                            .insert(validator_address.clone(), validator.balance);
                    }

                    accounts_tree.put(
                        db_txn,
                        &StakingContract::get_key_validator(validator_address),
                        Account::StakingValidator(validator),
                    );

                    accounts_tree.put(
                        db_txn,
                        &StakingContract::get_key_validator_staker(
                            validator_address,
                            staker_address,
                        ),
                        Account::StakingValidatorsStaker(staker_address.clone()),
                    );
                }

                Staker {
                    address: staker_address.clone(),
                    balance: value,
                    delegation: receipt.delegation,
                }
            }
            None => {
                let mut staker = StakingContract::get_staker(accounts_tree, db_txn, staker_address)
                    .ok_or(AccountError::NonExistentAddress {
                        address: staker_address.clone(),
                    })?;

                staker.balance = Account::balance_add(staker.balance, value)?;

                staker
            }
        };

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staker(staker_address),
            Account::StakingStaker(staker.clone()),
        );

        staking_contract.balance = Account::balance_add(staking_contract.balance, value)?;

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(vec![Log::Unstake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation,
            value,
        }])
    }
}
