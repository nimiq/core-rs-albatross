#[cfg(feature = "interaction-traits")]
use std::cmp::Ordering;

use nimiq_keys::Address;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountError;
use nimiq_primitives::coin::Coin;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::policy::Policy;
use serde::{Deserialize, Serialize};

use crate::RetireStakeReceipt;
#[cfg(feature = "interaction-traits")]
use crate::{
    account::staking_contract::{
        store::{
            StakingContractStoreReadOps, StakingContractStoreReadOpsExt, StakingContractStoreWrite,
        },
        StakerReceipt, StakingContract, Tombstone,
    },
    Log, RemoveStakeReceipt, SetActiveStakeReceipt, TransactionLog,
};

/// Struct representing a staker in the staking contract.
/// The staker's balance is divided into active and inactive stake.
///
/// Actions concerning a staker are:
/// 1. Create:           Creates a staker.
/// 2. Stake:            Adds coins from any outside address to a staker's active balance.
/// 3. SetActiveStake:   Re-balances between active and inactive stake by setting the amount of active stake.
///                      This action restarts the lock-up period of the inactive stake.
///                      Inactive stake is locked up for other actions until the release block.
///                      If a delegation is defined and the corresponding validator is jailed,
///                      the maximum between the inactive balance lock-up period and the jail period applies.
/// 3. Update:           Updates the validator address the stake is delegated to.
///                      This action is only possible if:
///                        (a) the stake was previously not delegated (i.e., delegated to `None`)
///                        (b) the active balance is zero and the inactive balance has been released (*).
/// 4. RetireStake:      Permanently marks the given balance for future withdrawal. Only inactive funds can be retired.
///                      This action can only take effect if the inactive funds are already released (cooldown
///                      and potential jail have passed).
/// 5. RemoveStake:      Removes coins from a staker's balance to outside the staking contract.
///                      The retired balance can always be withdrawn as long as the staker has still minimum stake or
///                      no more funds. On the latter case, this action will result in the deletion of the staker.
///
/// Create, Stake, SetActiveStake, Update and RetireStake are incoming transactions to the staking contract.
/// Remove stake is an outgoing transaction from the staking contract.
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
    /// The block number at which the inactive balance was last inactivated
    /// If the stake is currently delegated to a jailed validator, the maximum of its jail release
    /// and the inactive release is taken. Re-delegation requires the whole balance of the staker to be inactive.
    /// The stake can only effectively become inactive on the next election block. Thus, this may contain a
    /// future block height.
    pub inactive_from: Option<u32>,
    /// The staker's retired balance. Retired balance can only be withdrawn, it is irreversible.
    /// Only released retired balance can be withdrawn from the staking contract.
    /// For retired balance to be available for withdrawal, the maximum of the inactive and the validator's jailed
    /// periods must have passed.
    pub retired_balance: Coin,
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
        inactive_balance: Coin,
        inactive_from: Option<u32>,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        self.create_staker_with_retired(
            store,
            staker_address,
            value,
            delegation,
            inactive_balance,
            inactive_from,
            Coin::ZERO,
            tx_logger,
        )
    }

    /// Creates a new staker with retired funds.
    fn create_staker_with_retired(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        delegation: Option<Address>,
        inactive_balance: Coin,
        inactive_from: Option<u32>,
        retired_balance: Coin,
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

        // The minimum stake is already checked during intrinsic transaction verification.
        // All checks passed, not allowed to fail from here on!

        // Create the staker struct.
        let staker = Staker {
            address: staker_address.clone(),
            balance: value,
            inactive_balance,
            retired_balance,
            inactive_from,
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

        // Update the staker's and staking contract's balances.
        // Update our balance.
        staker.balance += value;
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

        // If stake is currently delegated, than the active balance should be 0.
        if staker.delegation.is_some() {
            // Fail if there is active stake.
            // All stake must be moved to being inactive before re-delegating.
            if !staker.balance.is_zero() {
                debug!(
                    ?staker_address,
                    "Tried to update staker with active balance"
                );
                return Err(AccountError::InvalidForRecipient);
            }

            // Fail if the the funds are in cooldown or jailed.
            if !self.is_inactive_stake_released(store, &staker, block_number) {
                debug!(
                    ?staker_address,
                    "Tried to update staker with inactive balance locked (cooldown or jail)"
                );
                return Err(AccountError::InvalidForRecipient);
            };
        }

        // All checks passed, not allowed to fail from here on!

        // Create the receipt.
        let receipt = StakerReceipt {
            delegation: staker.delegation.clone(),
            active_balance: staker.balance,
            inactive_from: staker.inactive_from,
        };

        // Store old information for the log.
        let old_validator_address = staker.delegation.clone();

        // If we were delegating to a validator, we remove ourselves from it.
        // We allow updates only when the balance is zero (the staker's stake has been removed already)
        // or the delegation is None. Thus, we only need to update the validator's stakers counter.
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

        // Restore the previous balances and values.
        staker.balance = receipt.active_balance;
        staker.inactive_balance = total_balance - staker.balance;
        staker.inactive_from = receipt.inactive_from;

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Changes the active and consequentially the inactive balances of staker.
    /// The active balance will be set immediately.
    /// The inactive balance will be available to withdraw after a lock up period and, if applicable,
    /// until the jail period is finished.
    /// If the staker has already some inactive stake, the corresponding balance will be overwritten and
    /// the lock up period will be reset. This corresponds to editing the active stake balance
    /// and re-adjusting the inactive stake balance.
    pub fn set_active_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        new_active_balance: Coin,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<SetActiveStakeReceipt, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Fail if staker does not have sufficient funds.
        let total_balance = staker.balance + staker.inactive_balance;
        if total_balance < new_active_balance {
            return Err(AccountError::InsufficientFunds {
                needed: new_active_balance,
                balance: total_balance,
            });
        }

        // All checks passed, not allowed to fail from here on!

        // Store old values for receipt.
        let old_inactive_from = staker.inactive_from;
        let old_active_balance = staker.balance;
        let new_inactive_balance = total_balance - new_active_balance;

        // Update the staker's balance.
        staker.balance = new_active_balance;
        staker.inactive_balance = new_inactive_balance;
        staker.inactive_from = if new_inactive_balance.is_zero() {
            None
        } else {
            // The inactivation only takes effect after the next election block.
            Some(Policy::election_block_after(block_number))
        };

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

        tx_logger.push_log(Log::SetActiveStake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            active_balance: new_active_balance,
            inactive_balance: new_inactive_balance,
            inactive_from: staker.inactive_from,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(SetActiveStakeReceipt {
            old_inactive_from,
            old_active_balance,
        })
    }

    /// Reverts a deactivate stake transaction.
    pub fn revert_set_active_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        receipt: SetActiveStakeReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        let total_balance = staker.balance + staker.inactive_balance;

        // Keep the old values.
        let old_inactive_from = staker.inactive_from;
        let old_inactive_balance = staker.inactive_balance;

        // Restore the previous inactive since and balances.
        staker.inactive_from = receipt.old_inactive_from;
        staker.balance = receipt.old_active_balance;
        staker.inactive_balance = total_balance - staker.balance;

        // If we are delegating to a validator, we update the active stake of the validator.
        // This function never changes the staker's delegation, so the validator counter should not be updated.
        if let Some(validator_address) = &staker.delegation {
            self.update_stake_for_validator(store, validator_address, value, staker.balance);
        }

        // Create logs.
        tx_logger.push_log(Log::SetActiveStake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            active_balance: value,
            inactive_balance: old_inactive_balance,
            inactive_from: old_inactive_from,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Adds to the retired balance and consequentially changes the inactive balance of the staker.
    /// The balance can only be retired if the lock up period and associated validator's jail period has passed.
    /// Once retired the funds can be withdrawn immediately as long as the min stake is ensured.
    pub fn retire_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<RetireStakeReceipt, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Fail if staker does not have sufficient funds to retire.
        if staker.inactive_balance < value {
            return Err(AccountError::InsufficientFunds {
                needed: value,
                balance: staker.inactive_balance,
            });
        }

        if !self.is_inactive_stake_released(store, &staker, block_number) {
            debug!(
                ?staker_address,
                "Tried to retire stake with inactive balance locked (cooldown or jail)"
            );
            return Err(AccountError::InvalidForRecipient);
        }

        // All checks passed, not allowed to fail from here on!

        // Store old values for receipt.
        let old_inactive_balance = staker.inactive_balance;
        let old_inactive_from = staker.inactive_from;

        // Update the staker's balances.
        staker.inactive_balance -= value;
        staker.retired_balance += value;

        if staker.inactive_balance.is_zero() {
            staker.inactive_from = None;
        }

        tx_logger.push_log(Log::RetireStake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            inactive_balance: staker.inactive_balance,
            inactive_from: staker.inactive_from,
            retired_balance: staker.retired_balance,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(RetireStakeReceipt {
            old_inactive_balance,
            old_inactive_from,
        })
    }

    /// Reverts a retire stake transaction.
    pub fn revert_retire_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        receipt: RetireStakeReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Keep the old values.
        let old_retire_balance = staker.retired_balance;
        let old_inactive_balance = staker.inactive_balance;
        let old_inactive_from = staker.inactive_from;

        // Restore the previous values.
        staker.retired_balance -= value;
        staker.inactive_balance = receipt.old_inactive_balance;
        staker.inactive_from = receipt.old_inactive_from;

        // Create logs.
        tx_logger.push_log(Log::RetireStake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            inactive_balance: old_inactive_balance,
            inactive_from: old_inactive_from,
            retired_balance: old_retire_balance,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Removes coins from the retire staker's balance. If the entire staker's balance is removed then the
    /// staker is deleted.
    pub fn remove_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        tx_logger: &mut TransactionLog,
    ) -> Result<RemoveStakeReceipt, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        self.can_remove_stake(&staker, value)?;

        // All checks passed, not allowed to fail from here on!

        // Keep the old values.
        let old_delegation = staker.delegation.clone();

        // Update balances.
        staker.retired_balance -= value;
        self.balance -= value;

        tx_logger.push_log(Log::RemoveStake {
            staker_address: staker_address.clone(),
            validator_address: old_delegation.clone(),
            value,
        });

        // Update or remove the staker entry, depending on remaining balance.
        // We do not update the validator's stake balance because remove stake
        // is only referring to already inactivated or retired stake. Thus this stake
        // has already been removed stake balance (from the validator).
        if staker.balance.is_zero()
            && staker.inactive_balance.is_zero()
            && staker.retired_balance.is_zero()
        {
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
        })
    }

    pub(crate) fn can_remove_stake(
        &self,
        staker: &Staker,
        value: Coin,
    ) -> Result<(), AccountError> {
        // Fail if the value is too high.
        let new_retired_balance = staker.retired_balance.safe_sub(value)?;

        // Fail if this operation would leave a staker with a total balance violating the minimum stake.
        // However, if this operation will result in a removal of the staker, it is allowed.
        let non_retired_balances = staker.balance + staker.inactive_balance;
        if non_retired_balances + new_retired_balance
            < Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
            && non_retired_balances + new_retired_balance > Coin::ZERO
        {
            debug!(
                active_balance=?staker.balance,
                inactive_balance=?staker.inactive_balance,
                retired_balance=?staker.retired_balance,
                ?value,
                "Tried to remove stake that would violate the minimum total stake"
            );
            return Err(AccountError::InvalidCoinValue);
        }

        Ok(())
    }

    fn is_inactive_stake_released(
        &self,
        store: &StakingContractStoreWrite,
        staker: &Staker,
        block_number: u32,
    ) -> bool {
        // If stake is currently delegated we check if funds are released.
        if let Some(validator_address) = &staker.delegation {
            // Funds are not released if release block height has not passed yet.
            if let Some(inactive_from) = staker.inactive_from {
                if block_number < Policy::block_after_reporting_window(inactive_from) {
                    return false;
                }
            }

            // Funds are only released if the validator is not jailed.
            if let Some(validator) = store.get_validator(validator_address) {
                return !validator.is_jailed(block_number);
            }
        }

        true
    }

    /*  Private functions */

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
            // Set the retired balance to zero here, it is updated later.
            Staker {
                address: staker_address.clone(),
                balance: Coin::ZERO,
                inactive_balance: Coin::ZERO,
                inactive_from: None,
                retired_balance: Coin::ZERO,
                delegation: receipt.delegation,
            }
        });

        // Update the staking contract's and staker's balances.
        staker.retired_balance += value;
        self.balance += value;

        tx_logger.push_log(Log::RemoveStake {
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
    pub(crate) fn unregister_staker_from_validator(
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
