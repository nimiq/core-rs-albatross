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
    DeleteStakerReceipt, Log, RetireStakeReceipt, SetActiveStakeReceipt, TransactionLog,
};

/// Struct representing a staker in the staking contract.
/// The staker's balance is divided into active, inactive and retired stake.
///
/// Actions concerning a staker are:
/// 1. Create:           Creates a staker.
/// 2. AddStake:         Adds coins from any outside address to the staker's active balance.
///                      This action is only possible if:
///                        (a) the resulting non-retired funds respect the invariant 1 - minimum stake for non-retired funds.
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
///                      This action is only possible if:
///                        (a) the resulting non-retired funds respect the invariant 1 - minimum stake for non-retired funds.
///                        (b) the inactive funds are released (*).
/// 5. RemoveStake:      Removes the retired balance from a staker to outside of the staking contract.
///                      The retired balance can always be withdrawn as long as it removes all retired stake.
///                      If the staker's total balance drops to 0, the staker is deleted.
///
/// (*)     For inactive balance to be released, the maximum of the lock-up period for inactive stake and the validator's
///         potential jail period must have passed.
/// (**)    The staker has a set of invariants:
///             (invariant 1) Min stake for non-retired balances:
///                           active + inactive balance must be == 0 or >= minimum stake
///             (invariant 2) Min stake for total staker balances:
///                           active + inactive + retired balance must be == 0 or >= minimum stake
///             (invariant 3) Inactivation block height is always associated with some inactive balance:
///                          `inactive_from` is Some(_) && `inactive_balance` > 0
///             (invariant 4) When there are no inactive funds, there is no inactivation block height:
///                          `inactive_from` is None && `inactive_balance` == 0
///
/// Create, AddStake, SetActiveStake, Update and RetireStake are incoming transactions to the staking contract.
/// RemoveStake is an outgoing transaction from the staking contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staker {
    /// The address of the staker. The corresponding key is used for all transactions (except AddStake
    /// which is open to any address).
    pub address: Address,
    /// The staker's active balance.
    pub active_balance: Coin,
    /// The staker's inactive balance. Only released inactive balance can be withdrawn from the staking contract.
    /// Stake can only be re-delegated if the whole balance of the staker is inactive and released
    /// (or if there was no prior delegation). For inactive balance to be released, the maximum of
    /// the inactive and the validator's jailed periods must have passed.
    pub inactive_balance: Coin,
    /// The block number at which the inactive balance was last inactivated.
    /// If the stake is currently delegated to a jailed validator, the maximum of its jail release
    /// and the inactive release is taken. Re-delegation requires the whole balance of the staker to be inactive.
    /// The stake can only effectively become inactive on the next election block. Thus, this may contain a
    /// future block height.
    pub inactive_from: Option<u32>,
    /// The staker's retired balance. Retired balance can only be withdrawn, thus retiring is irreversible.
    /// Only released inactive balance can be retired, so the maximum of the inactive and the validator's jailed
    /// periods must have passed.
    /// Once retired, the funds are immediately available to be withdrawn (removed).
    pub retired_balance: Coin,
    /// The address of the validator for which the staker is delegating its stake for. If it is not
    /// delegating to any validator, this will be set to None.
    pub delegation: Option<Address>,
}

#[cfg(feature = "interaction-traits")]
impl Staker {
    /// Returns the current total balance of the staker.
    pub fn total_balance(&self) -> Coin {
        self.active_balance + self.inactive_balance + self.retired_balance
    }

    /// Returns the current non-retired balance of the staker.
    pub fn none_retired_balance(&self) -> Coin {
        self.active_balance + self.inactive_balance
    }

    /// Checks that the given value can be withdrawn from the retired balance.
    /// Fails if we are trying to withdraw less than total retired balance.
    pub(crate) fn can_remove_stake(&self, value: Coin) -> Result<(), AccountError> {
        // The value to be withdrawn must be equal to the retired balance.
        if value != self.retired_balance {
            return Err(AccountError::InvalidCoinValue);
        }

        Ok(())
    }

    /*  Helpers for checking release block heights and invariants */

    /// Returns true if the inactive funds are released both from the lockup period and any potential jail
    /// of the current validator delegation.
    fn is_inactive_stake_released(
        &self,
        store: &StakingContractStoreWrite,
        block_number: u32,
    ) -> bool {
        // If stake is currently delegated we check if funds are released.
        // We only need to check the release of inactive funds if there is a delegation.
        // This is because the purpose of the locked period is to ensure that we wait for the reporting window
        // to finish and thus that the validator will not be jailed due to misbehavior.
        if let Some(validator_address) = &self.delegation {
            // Funds are not released if release block height has not passed yet.
            if let Some(inactive_from) = self.inactive_from {
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

    /// Checks if the minimum stake invariants are respected.
    /// This function expects as input the final balances of a given operation.
    /// Invariants:
    ///         (1) active + inactive balances must be == 0 or >= minimum stake
    ///         (2) active + inactive + retired balances must be == 0 or >= minimum stake
    pub(crate) fn enforce_min_stake(
        active_balance: Coin,
        inactive_balance: Coin,
        retired_balance: Coin,
    ) -> Result<(), AccountError> {
        // Check invariant 1: that total non-retired balances (active+inactive) do not violate the minimum stake.
        let non_retired_balance = active_balance + inactive_balance;
        if non_retired_balance > Coin::ZERO
            && non_retired_balance < Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
        {
            debug!(
                ?active_balance,
                ?inactive_balance,
                "Transaction would violate the minimum non-retired stake"
            );
            return Err(AccountError::InvalidCoinValue);
        }

        // Check invariant 2: total stake does not violate minimum stake.
        // If all funds are retired, then retired funds must be 0 or at least minimum stake.
        if non_retired_balance.is_zero()
            && retired_balance > Coin::ZERO
            && retired_balance < Coin::from_u64_unchecked(Policy::MINIMUM_STAKE)
        {
            debug!(
                ?active_balance,
                ?inactive_balance,
                ?retired_balance,
                "Transaction would violate the minimum total stake"
            );
            return Err(AccountError::InvalidCoinValue);
        }

        Ok(())
    }
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
            active_balance: value,
            inactive_balance,
            retired_balance: Coin::ZERO,
            inactive_from,
            delegation,
        };

        // If we are delegating to a validator, we need to update it.
        if let Some(validator_address) = &staker.delegation {
            self.register_staker_on_validator(store, validator_address, false);
            self.increase_stake_to_validator(store, validator_address, staker.active_balance);
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
        assert_eq!(value, staker.active_balance);
        self.balance -= value;

        // If we are delegating to a validator, we need to update it.
        if let Some(validator_address) = &staker.delegation {
            self.decrease_stake_from_validator(store, validator_address, staker.active_balance);
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

        // Fail if the minimum stake would be violated for the non-retired funds (invariant 1).
        Staker::enforce_min_stake(
            staker.active_balance + value,
            staker.inactive_balance,
            staker.retired_balance,
        )?;

        // All checks passed, not allowed to fail from here on!

        // If we are delegating to a validator, we need to update it.
        if let Some(validator_address) = &staker.delegation {
            // Check that the delegation is still valid, i.e. the validator hasn't been deleted.
            store.expect_validator(validator_address)?;
            self.increase_stake_to_validator(store, validator_address, value);
        }

        // Update the staker's and staking contract's balances.
        staker.active_balance += value;
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

        // Update the staker's and staking contract's balances.
        staker.active_balance -= value;
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

        // If stake is currently delegated, the active balance must be 0.
        if staker.delegation.is_some() {
            // Fail if there is active stake.
            // All stake must be inactivated or retired before re-delegating.
            if !staker.active_balance.is_zero() {
                debug!(
                    ?staker_address,
                    "Tried to update staker with active balance"
                );
                return Err(AccountError::InvalidForRecipient);
            }

            // Fail if the funds are locked or jailed.
            if !staker.is_inactive_stake_released(store, block_number) {
                debug!(
                    ?staker_address,
                    "Tried to update staker with inactive balance locked or jailed"
                );
                return Err(AccountError::InvalidForRecipient);
            };
        }

        // All checks passed, not allowed to fail from here on!

        // Create the receipt.
        let receipt = StakerReceipt {
            delegation: staker.delegation.clone(),
            active_balance: staker.active_balance,
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
            staker.active_balance += staker.inactive_balance;
            staker.inactive_balance = Coin::ZERO;
            staker.inactive_from = None;
        }

        // If we are now delegating to a validator we add ourselves to it.
        if let Some(validator_address) = &staker.delegation {
            self.register_staker_on_validator(store, validator_address, false);
            if !staker.active_balance.is_zero() {
                self.increase_stake_to_validator(store, validator_address, staker.active_balance);
            }
        }

        // Create log
        tx_logger.push_log(Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address,
            new_validator_address: staker.delegation.clone(),
            active_balance: staker.active_balance,
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

        // Remove ourselves from the current delegation, if it exists.
        if let Some(validator_address) = &staker.delegation {
            if !staker.active_balance.is_zero() {
                self.decrease_stake_from_validator(store, validator_address, staker.active_balance)
            }
            self.unregister_staker_from_validator(store, validator_address);
        }

        // Create logs.
        tx_logger.push_log(Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: receipt.delegation.clone(),
            new_validator_address: staker.delegation.clone(),
            active_balance: staker.active_balance,
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
        let total_balance = staker.total_balance();
        staker.active_balance = receipt.active_balance;
        staker.inactive_balance = total_balance - staker.active_balance;
        staker.inactive_from = receipt.inactive_from;

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Changes the active and consequentially the inactive balances of the staker.
    /// The active balance will be set immediately.
    /// The inactive balance will be available to retire after a lock-up period and, if applicable,
    /// after the validator's jail period has finished.
    /// If the staker already has some inactive stake, the corresponding balance will be overwritten and
    /// the lock-up period will be reset. This corresponds to editing the active stake balance
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
        let total_balance = staker.total_balance();
        if total_balance < new_active_balance {
            return Err(AccountError::InsufficientFunds {
                needed: new_active_balance,
                balance: total_balance,
            });
        }

        // All checks passed, not allowed to fail from here on!

        // Store old values for receipt.
        let old_inactive_from = staker.inactive_from;
        let old_active_balance = staker.active_balance;
        let new_inactive_balance = total_balance - new_active_balance;

        // Update the staker's balances.
        staker.active_balance = new_active_balance;
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

        // Keep the old values.
        let total_balance = staker.total_balance();
        let old_inactive_from = staker.inactive_from;
        let old_inactive_balance = staker.inactive_balance;

        // Restore the previous inactive since and balances.
        staker.inactive_from = receipt.old_inactive_from;
        staker.active_balance = receipt.old_active_balance;
        staker.inactive_balance = total_balance - staker.active_balance;

        // If we are delegating to a validator, we update the active stake of the validator.
        // This function never changes the staker's delegation, so the validator counter should not be updated.
        if let Some(validator_address) = &staker.delegation {
            self.update_stake_for_validator(store, validator_address, value, staker.active_balance);
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
    /// The balance can only be retired if the lock-up period and associated validator's jail period have passed.
    /// The retire fails if the invariant 1 for the non-retired stake is violated.
    /// Once retired the funds can be withdrawn immediately.
    pub fn retire_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        retire_stake: Coin,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<RetireStakeReceipt, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Fail if staker does not have sufficient funds to retire.
        if staker.inactive_balance < retire_stake {
            return Err(AccountError::InsufficientFunds {
                needed: retire_stake,
                balance: staker.inactive_balance,
            });
        }

        // Fail if the minimum stake would be violated for non-retired funds (invariant 1).
        let new_inactive_balance = staker.inactive_balance - retire_stake;
        Staker::enforce_min_stake(
            staker.active_balance,
            new_inactive_balance,
            staker.retired_balance + retire_stake,
        )?;

        // Fail if the funds are locked or jailed.
        if !staker.is_inactive_stake_released(store, block_number) {
            debug!(
                ?staker_address,
                "Tried to retire stake with inactive balance locked or jailed"
            );
            return Err(AccountError::InvalidForRecipient);
        }

        // All checks passed, not allowed to fail from here on!

        // Store old values for receipt.
        let old_inactive_from = staker.inactive_from;

        // Update the staker's balances.
        staker.inactive_balance = new_inactive_balance;
        staker.retired_balance += retire_stake;

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

        Ok(RetireStakeReceipt { old_inactive_from })
    }

    /// Reverts a retire stake transaction.
    pub fn revert_retire_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        retire_stake: Coin,
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
        staker.retired_balance -= retire_stake;
        staker.inactive_balance += retire_stake;
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

    /// Removes balance from the retired stake of the staker.
    /// The stake removal fails if the invariant 2 for minimum stake for total balance is violated.
    /// If the staker's total balance is 0 then the staker is deleted and a receipt is returned.
    /// IMPORTANT: The remove stake should always remove the total stake. This is not enforced on
    /// this level to allow for the fee deduction in case of commit failed transactions. Thus, upon
    /// committing a regular remove stake transaction, the `can_remove_stake` must be called prior
    /// the stake removal.
    pub fn remove_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<DeleteStakerReceipt>, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Fails if there are not enough retired funds for withdrawal.
        if value > staker.retired_balance {
            return Err(AccountError::InsufficientFunds {
                needed: value,
                balance: staker.retired_balance,
            });
        }

        // Fails if the minimum total stake is violated (invariant 2).
        let new_retired_balance = staker.retired_balance - value;
        Staker::enforce_min_stake(
            staker.active_balance,
            staker.inactive_balance,
            new_retired_balance,
        )?;

        // All checks passed, not allowed to fail from here on!

        // Update balances.
        staker.retired_balance = new_retired_balance;
        self.balance -= value;

        tx_logger.push_log(Log::RemoveStake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
        });

        // Update or remove the staker entry, depending on remaining total balance.
        // We do not update the validator's stake balance because remove stake
        // is only referring to already retired stake. Thus this stake
        // has already been removed from the validator.
        let receipt = if staker.total_balance().is_zero() {
            // If staker is to be removed and it had delegation, we update the validator.
            if let Some(validator_address) = &staker.delegation {
                self.unregister_staker_from_validator(store, validator_address);
            }
            store.remove_staker(staker_address);

            tx_logger.push_log(Log::DeleteStaker {
                staker_address: staker_address.clone(),
                validator_address: staker.delegation.clone(),
            });

            Some(DeleteStakerReceipt {
                delegation: staker.delegation,
            })
        } else {
            store.put_staker(staker_address, staker);

            None
        };

        Ok(receipt)
    }

    /// Reverts a remove_stake transaction.
    pub fn revert_remove_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        receipt: Option<DeleteStakerReceipt>,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        let staker = if let Some(receipt) = receipt {
            if let Some(validator_address) = &receipt.delegation {
                self.register_staker_on_validator(store, validator_address, true);
            }

            tx_logger.push_log(Log::DeleteStaker {
                staker_address: staker_address.clone(),
                validator_address: receipt.delegation.clone(),
            });

            Staker {
                address: staker_address.clone(),
                active_balance: Coin::ZERO,
                inactive_balance: Coin::ZERO,
                inactive_from: None,
                retired_balance: value,
                delegation: receipt.delegation,
            }
        } else {
            // If there is no receipt the staker must exist and thus we only need to update the balance.
            let mut staker = store.expect_staker(staker_address)?;
            staker.retired_balance += value;

            staker
        };

        // Update the staking contract's balance.
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

    /* Helpers for delegation counter and balance update */

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

    /// Wrapper for decreasing or increasing the validator's balance.
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
