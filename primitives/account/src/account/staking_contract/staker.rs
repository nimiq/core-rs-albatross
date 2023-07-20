use std::cmp::Ordering;

use nimiq_keys::Address;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountError;
use nimiq_primitives::{coin::Coin, policy::Policy};
use serde::{Deserialize, Serialize};

#[cfg(feature = "interaction-traits")]
use crate::{
    account::staking_contract::{
        store::{
            StakingContractStoreReadOps, StakingContractStoreReadOpsExt, StakingContractStoreWrite,
        },
        StakerReceipt, StakingContract, Tombstone,
    },
    Log, TransactionLog,
};
use crate::{RemoveStakeReceipt, SetInactiveStakeReceipt};

/// Struct representing a staker in the staking contract.
/// The staker's balance is divided into active and inactive stake.
///
/// Actions concerning a staker are:
/// 1. Create:           Creates a staker.
/// 2. Stake:            Adds coins from any outside address to a staker's active balance.
/// 3. SetInactiveStake: Re-balances between active and inactive stake by setting the amount of inactive stake.
///                      This action restarts the lock-up period of the inactive stake.
///                      Inactive stake is locked up for other actions until the release block.
///                      If a delegation is defined and the corresponding validator is jailed,
///                      the maximum between the inactive balance lock-up period and the jail period applies.
/// 3. Update:           Updates the validator address the stake is delegated to.
///                      This action is only possible if:
///                        (a) the stake was previously not delegated (i.e., delegated to `None`)
///                        (b) the active balance is zero and the inactive balance has been released (*).
/// 4. Unstake:          Removes coins from a staker's balance to outside the staking contract.
///                      Only inactive stake that has been released can be withdrawn (*).
///
/// (*) For inactive balance to be released, the maximum of the lock-up period for inactive stake
///     and the validator's potential jail period must have passed.
///
/// Create, Stake, SetInactiveStake, and Update are incoming transactions to the staking contract.
/// Unstake is an outgoing transaction from the staking contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staker {
    /// The address of the staker. The corresponding key is used for all transactions (except Stake
    /// which is open to any address).
    pub address: Address,
    /// The staker's active balance.
    pub balance: Coin,
    /// The staker's inactive balance.
    pub inactive_balance: Coin,
    /// The inactive balance release block height. The effective release block height
    /// is also affected by the delegation jail release.
    pub inactive_release: Option<u32>,
    /// The address of the validator for which the staker is delegating its stake for. If it is not
    /// delegating to any validator, this will be set to None.
    pub delegation: Option<Address>,
}

#[cfg(feature = "interaction-traits")]
impl StakingContract {
    /// Creates a new staker. This function is public to fill the genesis staking contract.
    pub fn create_staker(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        delegation: Option<Address>,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // See if the staker already exists.
        if store.get_staker(staker_address).is_some() {
            return Err(AccountError::AlreadyExistentAddress {
                address: staker_address.clone(),
            });
        }

        // Check that the delegated validator exists.
        if let Some(validator_address) = &delegation {
            store.expect_validator(validator_address)?;
        }

        // Create the staker struct.
        let staker = Staker {
            address: staker_address.clone(),
            balance: value,
            inactive_balance: Coin::ZERO,
            inactive_release: None,
            delegation,
        };

        // If we are delegating to a validator, we need to update it.
        if staker.delegation.is_some() {
            self.add_staker_to_validator(store, &staker)?;
        }

        // Update balance.
        self.balance += value;

        // Build the return logs
        tx_logger.push_log(Log::CreateStaker {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
        });

        // Add the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Reverts a create staker transaction.
    pub fn revert_create_staker(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the staker.
        let staker = store.expect_staker(staker_address)?;

        // Update our balance.
        assert_eq!(value, staker.balance);
        self.balance -= value;

        // If we are delegating to a validator, we need to update it.
        if staker.delegation.is_some() {
            self.remove_staker_from_validator(store, &staker)
                .expect("inconsistent contract state");
        }

        // Remove the staker entry.
        store.remove_staker(staker_address);

        tx_logger.push_log(Log::CreateStaker {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
        });

        Ok(())
    }

    /// Adds more Coins to a staker's balance. It will be directly added to the staker's balance.
    /// Anyone can add stake for a staker. The staker must already exist.
    pub fn add_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // If we are delegating to a validator, we need to update it.
        if let Some(validator_address) = &staker.delegation {
            // Check that the delegation is still valid, i.e. the validator hasn't been deleted.
            store.expect_validator(validator_address)?;

            // Update the validator.
            self.increase_stake_to_validator(store, validator_address, value)?;
        }

        // Update the staker's balance.
        staker.balance += value;

        // Update our balance.
        self.balance += value;

        // Build the return logs
        tx_logger.push_log(Log::Stake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Reverts a stake transaction.
    pub fn revert_add_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // If we are delegating to a validator, we need to update it too.
        if let Some(validator_address) = &staker.delegation {
            self.decrease_stake_from_validator(store, validator_address, value)
                .expect("inconsistent contract state");
        }

        // Update the staker's balance.
        staker.balance -= value;

        // Update our balance.
        self.balance -= value;

        tx_logger.push_log(Log::Stake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Updates the staker details. Right now you can only update the delegation.
    /// Can only performed if there is no active stake.
    pub fn update_staker(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        delegation: Option<Address>,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<StakerReceipt, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Check that the delegated validator exists.
        if let Some(new_validator_address) = &delegation {
            store.expect_validator(new_validator_address)?;
        }

        // If stake is currently delegated, it must be inactive.
        if let Some(validator_address) = &staker.delegation {
            // Fail if there is active stake.
            // All stake must be moved to being inactive before re-delegating.
            if !staker.balance.is_zero() {
                debug!(
                    ?staker_address,
                    "Tried to update staker with active balance"
                );
                return Err(AccountError::InvalidForRecipient);
            }

            // Fail if the release block height has not passed yet.
            if let Some(inactive_release) = staker.inactive_release {
                if block_number < inactive_release {
                    debug!(
                        ?staker_address,
                        "Tried to update staker with inactive balance not being released yet"
                    );
                    return Err(AccountError::InvalidForRecipient);
                }
            }

            // Fail if validator is currently jailed.
            if let Some(validator) = store.get_validator(validator_address) {
                if let Some(jail_release) = validator.jail_release {
                    if block_number < jail_release {
                        debug!(
                            ?staker_address,
                            "Tried to update staker that currently delegates to a jailed validator"
                        );
                        return Err(AccountError::InvalidForRecipient);
                    }
                }
            }
        }

        // All checks passed, not allowed to fail from here on!

        // Create logs.
        tx_logger.push_log(Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: staker.delegation.clone(),
            new_validator_address: delegation.clone(),
        });

        // Create the receipt.
        let receipt = StakerReceipt {
            delegation: staker.delegation.clone(),
        };

        // We allow updates only when the balance is zero (the staker has been removed already)
        // or the delegation is None. Thus, we do not need to update the source validator.

        // Update the staker's delegation.
        staker.delegation = delegation;

        // If we are now delegating to a validator and have a non-zero balance, we add ourselves to it.
        if staker.delegation.is_some() && !staker.balance.is_zero() {
            self.add_staker_to_validator(store, &staker)
                .expect("inconsistent contract state");
        }

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(receipt)
    }

    /// Reverts updating staker details.
    pub fn revert_update_staker(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        receipt: StakerReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Remove ourselves from the current delegation, if it exists.
        if staker.delegation.is_some() && !staker.balance.is_zero() {
            self.remove_staker_from_validator(store, &staker)
                .expect("inconsistent contract state");
        }

        // Create logs.
        tx_logger.push_log(Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: receipt.delegation.clone(),
            new_validator_address: staker.delegation.clone(),
        });

        // Restore the previous delegation.
        staker.delegation = receipt.delegation;

        // We do not need to add the staker to the previous validator because
        // either delegation is None or there is no active stake.

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Deactivates a portion of the coins from a staker's balance. The inactive balance
    /// will be available to unstake after a lock up period and, if applicable,
    /// until the jail period is finished.
    /// If the staker has already inactive stake, the corresponding balance will be overwritten and
    /// the lock up period will be reset. This corresponds to editing the inactive stake balance
    /// and re-adjusting the active stake balance.
    pub fn set_inactive_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        new_inactive_balance: Coin,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<SetInactiveStakeReceipt, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Fail if staker does not have sufficient funds.
        let total_balance = staker.balance + staker.inactive_balance;
        if total_balance < new_inactive_balance {
            return Err(AccountError::InsufficientFunds {
                needed: new_inactive_balance,
                balance: total_balance,
            });
        }

        // All checks passed, not allowed to fail from here on!

        // Store old values for receipt.
        let old_inactive_release = staker.inactive_release;
        let old_active_balance = staker.balance;
        let new_active_balance = total_balance - new_inactive_balance;

        // If we are delegating to a validator, we update it.
        if let Some(validator_address) = &staker.delegation {
            if new_active_balance.is_zero() {
                self.remove_staker_from_validator(store, &staker)
                    .expect("inconsistent contract state");
            } else {
                self.update_stake_for_validator(
                    store,
                    validator_address,
                    old_active_balance,
                    new_active_balance,
                )
                .expect("inconsistent contract state");
            }
        }

        // Update the staker's balance.
        staker.balance = new_active_balance;
        staker.inactive_balance = new_inactive_balance;
        staker.inactive_release = if new_inactive_balance.is_zero() {
            None
        } else {
            // We release after the end of the reporting window.
            Some(Policy::block_after_reporting_window(
                Policy::election_block_after(block_number),
            ))
        };

        tx_logger.push_log(Log::SetInactiveStake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value: new_inactive_balance,
            inactive_release: staker.inactive_release,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(SetInactiveStakeReceipt {
            old_inactive_release,
            old_active_balance,
        })
    }

    /// Reverts a deactivate stake transaction.
    pub fn revert_set_inactive_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        receipt: SetInactiveStakeReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        let total_balance = staker.balance + staker.inactive_balance;

        // Keep the old values.
        let old_inactive_release = staker.inactive_release;
        let old_balance = staker.balance;

        // Restore the previous inactive release and balances.
        staker.inactive_release = receipt.old_inactive_release;
        staker.balance = receipt.old_active_balance;
        staker.inactive_balance = total_balance - staker.balance;

        // If we are delegating to a validator, we update it.
        if let Some(validator_address) = &staker.delegation {
            // If the staker has previously been removed, we add it back.
            if old_balance.is_zero() {
                self.add_staker_to_validator(store, &staker)
                    .expect("inconsistent contract state");
            } else {
                // Update the active stake of the validator.
                self.update_stake_for_validator(
                    store,
                    validator_address,
                    old_balance,
                    staker.balance,
                )
                .expect("inconsistent contract state");
            }
        }

        // Create logs.
        tx_logger.push_log(Log::SetInactiveStake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
            inactive_release: old_inactive_release,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Removes coins from a staker's balance. If the entire staker's balance is removed then the
    /// staker is deleted.
    pub fn remove_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<RemoveStakeReceipt, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Compute the new balance of the staker. We can't update `staker` here yet as
        // `remove_staker_from_validator` needs the original balance intact.
        let new_balance = staker.inactive_balance.safe_sub(value)?;

        // We only need to wait for the release time if stake is delegated.
        if let Some(validator_address) = &staker.delegation {
            // Fail if the release block height has not passed yet.
            if let Some(inactive_release) = staker.inactive_release {
                if block_number < inactive_release {
                    debug!(
                        ?staker_address,
                        "Tried to remove stake while the inactive balance has not been released yet"
                    );
                    return Err(AccountError::InvalidForSender);
                }
            }

            // Fail if validator is currently jailed.
            if let Some(validator) = store.get_validator(validator_address) {
                if let Some(jail_release) = validator.jail_release {
                    if block_number < jail_release {
                        debug!(
                            ?staker_address,
                            "Tried to remove stake that is currently delegated to a jailed validator"
                        );
                        return Err(AccountError::InvalidForSender);
                    }
                }
            }
        }

        // All checks passed, not allowed to fail from here on!

        // Keep the old values.
        let old_inactive_release = staker.inactive_release;
        let old_delegation = staker.delegation.clone();

        // Update the staker's balance.
        staker.inactive_balance = new_balance;
        if staker.inactive_balance.is_zero() {
            staker.inactive_release = None;
        }

        // Update our balance.
        self.balance -= value;

        tx_logger.push_log(Log::Unstake {
            staker_address: staker_address.clone(),
            validator_address: old_delegation.clone(),
            value,
        });

        // Update or remove the staker entry, depending on remaining balance.
        if staker.balance.is_zero() && staker.inactive_balance.is_zero() {
            store.remove_staker(staker_address);
        } else {
            store.put_staker(staker_address, staker);
        }

        Ok(RemoveStakeReceipt {
            delegation: old_delegation,
            inactive_release: old_inactive_release,
        })
    }

    /// Reverts a remove_stake transaction.
    pub fn revert_remove_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        receipt: RemoveStakeReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        let mut staker = store.get_staker(staker_address).unwrap_or_else(|| {
            // Set the staker balance to zero here, it is updated later.
            Staker {
                address: staker_address.clone(),
                balance: Coin::ZERO,
                inactive_balance: Coin::ZERO,
                inactive_release: None,
                delegation: receipt.delegation,
            }
        });

        // Update the staker's balance.
        staker.inactive_balance += value;
        // Update the staker's inactive release.
        staker.inactive_release = receipt.inactive_release;

        // Update our balance.
        self.balance += value;

        tx_logger.push_log(Log::Unstake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Adds a new staker to the validator given in staker.delegation.
    /// Panics if staker.delegation is None.
    fn add_staker_to_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker: &Staker,
    ) -> Result<(), AccountError> {
        let validator_address = staker
            .delegation
            .as_ref()
            .expect("Staker has no delegation");

        // Try to get the validator. It might have been deleted.
        if let Some(mut validator) = store.get_validator(validator_address) {
            // Validator exists, update it.
            validator.total_stake += staker.balance;

            if validator.is_active() {
                self.active_validators
                    .insert(validator_address.clone(), validator.total_stake);
            }

            validator.num_stakers += 1;

            // Update the validator entry.
            store.put_validator(validator_address, validator);

            return Ok(());
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.remaining_stake += staker.balance;
            tombstone.num_remaining_stakers += 1;

            store.put_tombstone(validator_address, tombstone);

            return Ok(());
        }

        // Tombstone doesn't exist, so it must have been deleted by a previous
        // `remove_staker_from_validator` call. Recreate it.
        // TODO We should consider guarding this functionality behind a flag. It's not obvious from
        //  the function name that this will create a tombstone if the validator doesn't exist.
        let tombstone = Tombstone {
            remaining_stake: staker.balance,
            num_remaining_stakers: 1,
        };
        store.put_tombstone(validator_address, tombstone);

        Ok(())
    }

    /// Removes a staker from the validator given in staker.delegation.
    /// Panics if staker.delegation is None.
    fn remove_staker_from_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker: &Staker,
    ) -> Result<(), AccountError> {
        let validator_address = staker
            .delegation
            .as_ref()
            .expect("Staker has no delegation");

        // Try to get the validator. It might have been deleted.
        if let Some(mut validator) = store.get_validator(validator_address) {
            // Validator exists, update it.
            validator.total_stake -= staker.balance;

            if validator.is_active() {
                self.active_validators
                    .insert(validator_address.clone(), validator.total_stake);
            }

            validator.num_stakers -= 1;

            // Update the validator entry.
            store.put_validator(validator_address, validator);

            return Ok(());
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.remaining_stake -= staker.balance;

            tombstone.num_remaining_stakers -= 1;

            // Delete the tombstone if this was the last remaining staker, update it otherwise.
            if tombstone.num_remaining_stakers == 0 {
                store.remove_tombstone(validator_address);
            } else {
                store.put_tombstone(validator_address, tombstone);
            }

            return Ok(());
        }

        // Neither validator nor tombstone exist, this is an error.
        panic!("inconsistent contract state");
    }

    /// Adds `value` coins to a given validator's total stake.
    fn increase_stake_to_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        value: Coin,
    ) -> Result<(), AccountError> {
        // Try to get the validator. It might have been deleted.
        if let Some(mut validator) = store.get_validator(validator_address) {
            // Validator exists, update it.
            validator.total_stake += value;

            if validator.is_active() {
                self.active_validators
                    .insert(validator_address.clone(), validator.total_stake);
            }

            // Update the validator entry.
            store.put_validator(validator_address, validator);

            return Ok(());
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.remaining_stake += value;

            // Update the tombstone entry.
            store.put_tombstone(validator_address, tombstone);

            return Ok(());
        }

        // Neither validator nor tombstone exist, this is an error.
        panic!("inconsistent contract state");
    }

    /// Removes `value` coins from a given validator's inactive total stake.
    fn decrease_stake_from_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        value: Coin,
    ) -> Result<(), AccountError> {
        // Try to get the validator. It might have been deleted.
        if let Some(mut validator) = store.get_validator(validator_address) {
            // Validator exists, update it.
            validator.total_stake -= value;

            if validator.is_active() {
                self.active_validators
                    .insert(validator_address.clone(), validator.total_stake);
            }

            // Update the validator entry.
            store.put_validator(validator_address, validator);

            return Ok(());
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.remaining_stake -= value;

            // Update the tombstone entry.
            store.put_tombstone(validator_address, tombstone);

            return Ok(());
        }

        // Neither validator nor tombstone exist, this is an error.
        panic!("inconsistent contract state");
    }

    fn update_stake_for_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        old_value: Coin,
        new_value: Coin,
    ) -> Result<(), AccountError> {
        match new_value.cmp(&old_value) {
            Ordering::Less => {
                self.decrease_stake_from_validator(
                    store,
                    validator_address,
                    old_value - new_value,
                )?;
            }
            Ordering::Greater => {
                self.increase_stake_to_validator(store, validator_address, new_value - old_value)?;
            }
            Ordering::Equal => {}
        }
        Ok(())
    }
}
