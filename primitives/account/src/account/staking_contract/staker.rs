#[cfg(feature = "interaction-traits")]
use std::cmp::Ordering;

use nimiq_keys::Address;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountError;
use nimiq_primitives::coin::Coin;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::policy::Policy;
use serde::{Deserialize, Serialize};

#[cfg(feature = "interaction-traits")]
use crate::{
    account::staking_contract::{
        store::{
            StakingContractStoreReadOps, StakingContractStoreReadOpsExt, StakingContractStoreWrite,
        },
        StakerReceipt, StakingContract, Tombstone,
    },
    Log, RemoveStakeReceipt, SetInactiveStakeReceipt, TransactionLog,
};

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
    /// The staker's inactive balance. Only released inactive balance can be withdrawn from the staking contract.
    /// Stake can only be re-delegated if the whole balance of the staker is inactive and released
    /// (or if there was no prior delegation). For inactive balance to be released, the maximum of
    /// the inactive and the validator's jailed periods must have passed.
    pub inactive_balance: Coin,
    /// The block number at which the inactive balance was last inactivated for withdrawal or re-delegation.
    /// If the stake is currently delegated to a jailed validator, the maximum of its jail release
    /// and the inactive release is taken. Re-delegation requires the whole balance of the staker to be inactive.
    /// The stake can only effectively become inactive on the next election block. Thus, this may contain a
    /// future block height.
    pub inactive_from: Option<u32>,
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
            inactive_from: None,
            delegation,
        };

        // If we are delegating to a validator, we need to update it.
        if let Some(validator_address) = &staker.delegation {
            self.register_staker_on_validator(store, validator_address, false);
            self.increase_stake_to_validator(store, validator_address, staker.balance);
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
        if let Some(validator_address) = &staker.delegation {
            self.decrease_stake_from_validator(store, validator_address, staker.balance);
            self.unregister_staker_from_validator(store, validator_address);
        }

        // Remove the staker entry.
        store.remove_staker(&staker.address);

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
            self.increase_stake_to_validator(store, validator_address, value);
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
            self.decrease_stake_from_validator(store, validator_address, value);
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

    /// Updates the staker details. Can update the delegation and the new inactive balance.
    /// Can only be performed if there is no active stake or no delegation.
    /// Inactive balance changes only take effect if the delegation can be updated.
    pub fn update_staker(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        new_delegation: Option<Address>,
        reactivate_all_stake: bool,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<StakerReceipt, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Check that the delegated validator exists.
        if let Some(new_validator_address) = &new_delegation {
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

            // Fail if the inactive release block height has not passed yet.
            if let Some(inactive_from) = staker.inactive_from {
                if block_number < Policy::block_after_reporting_window(inactive_from) {
                    debug!(
                        ?staker_address,
                        "Tried to update staker with inactive balance not being released yet"
                    );
                    return Err(AccountError::InvalidForRecipient);
                }
            }

            // Fail if validator is currently jailed.
            if let Some(validator) = store.get_validator(validator_address) {
                if validator.is_jailed(block_number) {
                    debug!(
                        ?staker_address,
                        "Tried to update staker that currently delegates to a jailed validator"
                    );
                    return Err(AccountError::InvalidForRecipient);
                }
            }
        }

        // All checks passed, not allowed to fail from here on!

        // Create the receipt.
        let receipt = StakerReceipt {
            delegation: staker.delegation.clone(),
            active_balance: staker.balance,
            inactive_from: staker.inactive_from,
        };

        // Store old information for the log
        let old_validator_address = staker.delegation.clone();

        // We allow updates only when the balance is zero (the staker's stake has been removed already)
        // or the delegation is None. Thus, we only need to update the validator's stakers counter.

        // If we were delegating to a validator, we remove ourselves from it.
        if let Some(validator_address) = &staker.delegation {
            self.unregister_staker_from_validator(store, validator_address);
        }

        // Update the staker's delegation.
        staker.delegation = new_delegation;

        if reactivate_all_stake {
            // Update the staker's balance.
            staker.balance += staker.inactive_balance;
            staker.inactive_balance = Coin::ZERO;
            staker.inactive_from = None;
        }

        // If we are now delegating to a validator we add ourselves to it.
        if let Some(validator_address) = &staker.delegation {
            self.register_staker_on_validator(store, validator_address, false);
            if !staker.balance.is_zero() {
                self.increase_stake_to_validator(store, validator_address, staker.balance);
            }
        }

        // Create log
        tx_logger.push_log(Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address,
            new_validator_address: staker.delegation.clone(),
            active_balance: staker.balance,
            inactive_from: staker.inactive_from,
        });

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
        let total_balance = staker.balance + staker.inactive_balance;

        // Remove ourselves from the current delegation, if it exists.
        if let Some(validator_address) = &staker.delegation {
            if !staker.balance.is_zero() {
                self.decrease_stake_from_validator(store, validator_address, staker.balance)
            }
            self.unregister_staker_from_validator(store, validator_address);
        }

        // Create logs.
        tx_logger.push_log(Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: receipt.delegation.clone(),
            new_validator_address: staker.delegation.clone(),
            active_balance: staker.balance,
            inactive_from: staker.inactive_from,
        });

        // Restore the previous delegation.
        staker.delegation = receipt.delegation;

        if let Some(validator_address) = &staker.delegation {
            self.register_staker_on_validator(store, validator_address, true);
            // when changing delegation there can only be a zero active stake,
            // so there is no need to update validator's stake.
        }

        // Restore the previous balances
        staker.balance = receipt.active_balance;
        staker.inactive_balance = total_balance - staker.balance;
        staker.inactive_from = receipt.inactive_from;

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
        let old_inactive_from = staker.inactive_from;
        let old_active_balance = staker.balance;
        let new_active_balance = total_balance - new_inactive_balance;

        // If we are delegating to a validator, we update the active stake of the validator.
        // This function never changes the staker's delegation, so the validator counter should not be updated.
        if let Some(validator_address) = &staker.delegation {
            self.update_stake_for_validator(
                store,
                validator_address,
                old_active_balance,
                new_active_balance,
            );
        }

        // Update the staker's balance.
        staker.balance = new_active_balance;
        staker.inactive_balance = new_inactive_balance;
        staker.inactive_from = if new_inactive_balance.is_zero() {
            None
        } else {
            // The inactivation only takes effect after the next election block.
            Some(Policy::election_block_after(block_number))
        };

        tx_logger.push_log(Log::SetInactiveStake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value: new_inactive_balance,
            inactive_from: staker.inactive_from,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(SetInactiveStakeReceipt {
            old_inactive_from,
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
        let old_inactive_from = staker.inactive_from;
        let old_balance = staker.balance;

        // Restore the previous inactive since and balances.
        staker.inactive_from = receipt.old_inactive_from;
        staker.balance = receipt.old_active_balance;
        staker.inactive_balance = total_balance - staker.balance;

        // If we are delegating to a validator, we update the active stake of the validator.
        // This function never changes the staker's delegation, so the validator counter should not be updated.
        if let Some(validator_address) = &staker.delegation {
            self.update_stake_for_validator(store, validator_address, old_balance, staker.balance);
        }

        // Create logs.
        tx_logger.push_log(Log::SetInactiveStake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
            inactive_from: old_inactive_from,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    pub(crate) fn can_remove_stake<S: StakingContractStoreReadOps>(
        &self,
        store: &S,
        staker: &Staker,
        block_number: u32,
    ) -> Result<(), AccountError> {
        // We only need to wait for the release time if stake is delegated.
        if let Some(validator_address) = &staker.delegation {
            // Fail if the inactive release block height has not passed yet.
            if let Some(inactive_from) = staker.inactive_from {
                if block_number < Policy::block_after_reporting_window(inactive_from) {
                    debug!(
                        ?staker.address,
                        "Tried to remove stake while the inactive balance has not been released yet"
                    );
                    return Err(AccountError::InvalidForSender);
                }
            }

            // Fail if validator is currently jailed.
            if let Some(validator) = store.get_validator(validator_address) {
                if validator.is_jailed(block_number) {
                    debug!(
                        ?staker.address,
                        "Tried to remove stake that is currently delegated to a jailed validator"
                    );
                    return Err(AccountError::InvalidForSender);
                }
            }
        }
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
        // `unregister_staker_from_validator` needs the original balance intact.
        let new_balance = staker.inactive_balance.safe_sub(value)?;

        self.can_remove_stake(store, &staker, block_number)?;

        // All checks passed, not allowed to fail from here on!

        // Keep the old values.
        let old_inactive_from = staker.inactive_from;
        let old_delegation = staker.delegation.clone();

        // Update the staker's balance.
        staker.inactive_balance = new_balance;
        if staker.inactive_balance.is_zero() {
            staker.inactive_from = None;
        }

        // Update our balance.
        self.balance -= value;

        tx_logger.push_log(Log::Unstake {
            staker_address: staker_address.clone(),
            validator_address: old_delegation.clone(),
            value,
        });

        // Update or remove the staker entry, depending on remaining balance.
        // We do not update the validator's stake balance because unstake
        // is only referring to already inactivated and thus already removed stake
        // balance (from the validator).
        if staker.balance.is_zero() && staker.inactive_balance.is_zero() {
            // If staker is to be removed and it had delegation, we update the validator.
            if let Some(validator_address) = &staker.delegation {
                self.unregister_staker_from_validator(store, validator_address);
            }
            store.remove_staker(staker_address);
        } else {
            store.put_staker(staker_address, staker);
        }

        Ok(RemoveStakeReceipt {
            delegation: old_delegation,
            inactive_from: old_inactive_from,
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
            // If staker had been removed and it had delegation, we update the validator.
            if let Some(validator_address) = &receipt.delegation {
                self.register_staker_on_validator(store, validator_address, true);
            }
            // Set the staker balance to zero here, it is updated later.
            Staker {
                address: staker_address.clone(),
                balance: Coin::ZERO,
                inactive_balance: Coin::ZERO,
                inactive_from: None,
                delegation: receipt.delegation,
            }
        });

        // Update the staker's balance.
        staker.inactive_balance += value;
        // Update the staker's `inactive_from` block height.
        staker.inactive_from = receipt.inactive_from;

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

    /// Registers/links a new staker on the validator.
    /// This only increases the counter of the validators' stakers, the balance is set with
    /// `increase_stake_to_validator`, `decrease_stake_to_validator` or `update_increase_stake_to_validator`.
    /// If the validator does not exist, it will create a tombstone.
    fn register_staker_on_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        can_create_tombstone: bool,
    ) {
        // Try to get the validator. It might have been deleted.
        if let Some(mut validator) = store.get_validator(validator_address) {
            // Validator exists, update it.
            validator.num_stakers += 1;
            store.put_validator(validator_address, validator);

            return;
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.num_remaining_stakers += 1;
            store.put_tombstone(validator_address, tombstone);

            return;
        }

        // Tombstone doesn't exist, so it must have been deleted by a previous
        // `unregister_staker_from_validator` call. Recreate it.
        // This is only needed in the reverts of the `update_staker`, `remove_stake`.
        if can_create_tombstone {
            let tombstone = Tombstone {
                remaining_stake: Coin::ZERO,
                num_remaining_stakers: 1,
            };
            store.put_tombstone(validator_address, tombstone);
        } else {
            // Neither validator nor tombstone exist, this is an error.
            panic!("inconsistent contract state");
        }
    }

    /// Un registers/links a new staker on the validator.
    /// This only increases the counter of the validators' stakers, the balance is set with
    /// `increase_stake_to_validator`, `decrease_stake_to_validator` or `update_increase_stake_to_validator`.
    /// In case of a deleted validator, this function removes the tombstone if there are no more stakers registered.
    /// Panics if staker.delegation is None.
    fn unregister_staker_from_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
    ) {
        // Try to get the validator. It might have been deleted.
        if let Some(mut validator) = store.get_validator(validator_address) {
            // Validator exists, update it.
            validator.num_stakers -= 1;
            store.put_validator(validator_address, validator);

            return;
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.num_remaining_stakers -= 1;

            // Delete the tombstone if this was the last remaining staker, update it otherwise.
            if tombstone.num_remaining_stakers == 0 {
                store.remove_tombstone(validator_address);
            } else {
                store.put_tombstone(validator_address, tombstone);
            }

            return;
        }

        // Neither validator nor tombstone exist, this is an error.
        panic!("inconsistent contract state");
    }

    fn update_stake_for_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        old_balance: Coin,
        new_balance: Coin,
    ) {
        match new_balance.cmp(&old_balance) {
            Ordering::Less => {
                self.decrease_stake_from_validator(
                    store,
                    validator_address,
                    old_balance - new_balance,
                );
            }
            Ordering::Greater => {
                self.increase_stake_to_validator(
                    store,
                    validator_address,
                    new_balance - old_balance,
                );
            }
            Ordering::Equal => {}
        }
    }

    /// Adds `value` coins to a given validator's total stake.
    fn increase_stake_to_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        value: Coin,
    ) {
        // Try to get the validator. It might have been deleted.
        if let Some(mut validator) = store.get_validator(validator_address) {
            // Validator exists, update it.
            validator.total_stake += value;
            if validator.is_active() {
                self.active_validators
                    .insert(validator_address.clone(), validator.total_stake);
            }
            store.put_validator(validator_address, validator);

            return;
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.remaining_stake += value;
            store.put_tombstone(validator_address, tombstone);

            return;
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
    ) {
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

            return;
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.remaining_stake -= value;
            // Update the tombstone entry.
            store.put_tombstone(validator_address, tombstone);

            return;
        }

        // Neither validator nor tombstone exist, this is an error.
        panic!("inconsistent contract state");
    }
}
