use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;

use beserial::{Deserialize, Serialize};
use bls::CompressedPublicKey as BlsPublicKey;
use keys::Address;
use primitives::coin::Coin;

use crate::staking_contract::actions::staker::InactiveStakeReceipt;
use crate::staking_contract::{InactiveValidator, Validator};
use crate::{Account, AccountError, StakingContract};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub(super) struct UnparkReceipt {
    current_epoch: bool,
    previous_epoch: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub(super) struct UpdateValidatorReceipt {
    old_reward_address: Address,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub(super) struct RetirementReceipt {
    stake: Coin,
    inactive_stake_receipt: Option<InactiveStakeReceipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub(super) struct DropValidatorReceipt {
    reward_address: Address,
    #[beserial(len_type(u32))]
    retirement_by_address: BTreeMap<Address, RetirementReceipt>,
    retire_time: u32,
    unpark_receipt: UnparkReceipt,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub(super) struct InactiveValidatorReceipt {
    retire_time: u32,
}

/// Actions concerning a validator are:
/// 1. Create: Creates a validator entry.
/// 2. Update: Updates reward address and key of the validator entry.
/// 3. Retire: Inactivates a validator entry (also starts a cooldown period used for Drop).
/// 4. Re-activate: Re-activates a validator entry.
/// 5. Drop: Drops a validator entry (validator must have been inactive for the cooldown period).
///          This also automatically retires the associated stake (allowing immediate withdrawal).
/// 6. Unpark: Prevents a validator entry from being automatically inactivated.
///
/// The actions can be summarized by the following state diagram:
///        +--------+   retire    +----------+
/// create |        +------------>+          | drop
///+------>+ active |             | inactive +------>
///        |        +<------------+          |
///        +-+--+---+ re-activate +-----+----+
///          |  ^                       ^
///          |  |                       |
/// mis-     |  | unpark                | automatically
/// behavior |  |                       |
///          |  |     +--------+        |
///          |  +-----+        |        |
///          |        | parked +--------+
///          +------->+        |
///                   +--------+
///
/// Create, Update, Retire, Re-activate, and Unpark are transactions from an arbitrary address
/// to the staking contract.
/// Drop is a transaction from the staking contract to an arbitrary address.
impl StakingContract {
    /// Creates a new validator entry.
    /// The initial stake can only be retrieved by dropping the validator again.
    /// XXX This is public to fill the genesis staking contract
    pub fn create_validator(
        &mut self,
        validator_key: BlsPublicKey,
        reward_address: Address,
        initial_stake: Coin,
    ) -> Result<(), AccountError> {
        if self.active_validators_by_key.contains_key(&validator_key)
            || self.inactive_validators_by_key.contains_key(&validator_key)
        {
            return Err(AccountError::InvalidForRecipient);
        }

        self.balance = Account::balance_add(self.balance, initial_stake)?;

        // All checks passed, not allowed to fail from here on!
        let validator = Arc::new(Validator::new(
            initial_stake,
            reward_address,
            validator_key.clone(),
        ));

        self.active_validators_sorted.insert(Arc::clone(&validator));
        self.active_validators_by_key
            .insert(validator_key, validator);
        Ok(())
    }

    /// Reverts creating a new validator entry.
    pub(super) fn revert_create_validator(
        &mut self,
        validator_key: BlsPublicKey,
        initial_stake: Coin,
    ) -> Result<(), AccountError> {
        if let Some(validator) = self.active_validators_by_key.remove(&validator_key) {
            self.balance = Account::balance_sub(self.balance, initial_stake)?;

            // All checks passed, not allowed to fail from here on!
            self.active_validators_sorted.remove(&validator);
            Ok(())
        } else {
            Err(AccountError::InvalidForRecipient)
        }
    }

    /// Update validator details.
    /// This can be used to update active and inactive validators.
    pub(super) fn update_validator(
        &mut self,
        old_validator_key: &BlsPublicKey,
        new_validator_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
    ) -> Result<UpdateValidatorReceipt, AccountError> {
        let mut entry = self
            .remove_validator(old_validator_key)
            .ok_or(AccountError::InvalidForRecipient)?;

        let old_reward_address = entry.as_validator().reward_address.clone();
        entry.update_validator(new_reward_address, new_validator_key);
        self.restore_validator(entry)?;

        Ok(UpdateValidatorReceipt { old_reward_address })
    }

    /// Reverts updating validator key.
    pub(super) fn revert_update_validator(
        &mut self,
        old_validator_key: BlsPublicKey,
        new_validator_key: Option<BlsPublicKey>,
        receipt: UpdateValidatorReceipt,
    ) -> Result<(), AccountError> {
        let mut entry = self
            .remove_validator(new_validator_key.as_ref().unwrap_or(&old_validator_key))
            .ok_or(AccountError::InvalidForRecipient)?;

        entry.update_validator(Some(receipt.old_reward_address), Some(old_validator_key));
        self.restore_validator(entry)?;

        Ok(())
    }

    /// Drops a validator entry.
    /// This can be used to drop inactive validators.
    /// The validator must have been inactive for at least one macro block.
    pub(super) fn drop_validator(
        &mut self,
        validator_key: &BlsPublicKey,
        _initial_stake: Coin,
    ) -> Result<DropValidatorReceipt, AccountError> {
        // Initial stake vs. stakes has been checked in check_outgoing_transaction.

        // All checks passed, not allowed to fail from here on!
        // Retire all stakes.
        let inactive_validator = self
            .inactive_validators_by_key
            .remove(validator_key)
            .unwrap();
        let validator = inactive_validator.validator;

        // We first remove all stakes the validator holds and will re-add stakes afterwards
        // when calling `retire_recipient`.
        self.balance = Account::balance_sub(self.balance, validator.balance)?;

        let mut retirement_by_address = BTreeMap::new();
        for (staker_address, &stake) in validator.active_stake_by_address.read().iter() {
            let receipt =
                self.retire_recipient(staker_address, stake, Some(inactive_validator.retire_time))?;
            retirement_by_address.insert(
                staker_address.clone(),
                RetirementReceipt {
                    stake,
                    inactive_stake_receipt: receipt,
                },
            );
        }

        // We need to check whether it is parked to prevent it from failing.
        let unpark_receipt = if self.current_epoch_parking.contains(validator_key)
            || self.previous_epoch_parking.contains(validator_key)
        {
            self.unpark_validator(validator_key)?
        } else {
            UnparkReceipt {
                current_epoch: false,
                previous_epoch: false,
            }
        };

        Ok(DropValidatorReceipt {
            reward_address: validator.reward_address.clone(),
            retirement_by_address,
            retire_time: inactive_validator.retire_time,
            unpark_receipt,
        })
    }

    /// Revert dropping a validator entry.
    pub(super) fn revert_drop_validator(
        &mut self,
        validator_key: BlsPublicKey,
        mut total_value: Coin,
        receipt: DropValidatorReceipt,
    ) -> Result<(), AccountError> {
        // First, revert retiring the stakers.
        let mut active_stake_by_address = BTreeMap::new();
        for (staker_address, receipt) in receipt.retirement_by_address {
            self.revert_retire_recipient(
                &staker_address,
                receipt.stake,
                receipt.inactive_stake_receipt,
            )?;
            active_stake_by_address.insert(staker_address, receipt.stake);
            total_value += receipt.stake;
        }

        self.balance = Account::balance_add(self.balance, total_value)?;

        // Cannot fail.
        self.revert_unpark_validator(&validator_key, receipt.unpark_receipt)?;

        self.inactive_validators_by_key.insert(
            validator_key.clone(),
            InactiveValidator {
                validator: Arc::new(Validator {
                    balance: total_value,
                    reward_address: receipt.reward_address,
                    validator_key,
                    active_stake_by_address: RwLock::new(active_stake_by_address),
                }),
                retire_time: receipt.retire_time,
            },
        );

        Ok(())
    }

    /// Inactivates a validator entry.
    pub(super) fn retire_validator(
        &mut self,
        validator_key: BlsPublicKey,
        block_height: u32,
    ) -> Result<(), AccountError> {
        // Move validator from active map/set to inactive map.
        let validator = self
            .active_validators_by_key
            .remove(&validator_key)
            .ok_or(AccountError::InvalidForRecipient)?;

        // All checks passed, not allowed to fail from here on!
        self.active_validators_sorted.remove(&validator);
        self.inactive_validators_by_key.insert(
            validator_key,
            InactiveValidator {
                validator,
                retire_time: block_height,
            },
        );
        Ok(())
    }

    /// Revert inactivating a validator entry.
    pub(super) fn revert_retire_validator(
        &mut self,
        validator_key: BlsPublicKey,
    ) -> Result<(), AccountError> {
        self.reactivate_validator(validator_key).map(|_| ())
    }

    /// Reactivate a validator entry.
    pub(super) fn reactivate_validator(
        &mut self,
        validator_key: BlsPublicKey,
    ) -> Result<InactiveValidatorReceipt, AccountError> {
        // Move validator from inactive map to active map/set.
        let inactive_validator = self
            .inactive_validators_by_key
            .remove(&validator_key)
            .ok_or(AccountError::InvalidForRecipient)?;

        // All checks passed, not allowed to fail from here on!
        self.active_validators_sorted
            .insert(Arc::clone(&inactive_validator.validator));
        self.active_validators_by_key
            .insert(validator_key, inactive_validator.validator);
        Ok(InactiveValidatorReceipt {
            retire_time: inactive_validator.retire_time,
        })
    }

    /// Inactivates a validator entry.
    pub(super) fn revert_reactivate_validator(
        &mut self,
        validator_key: BlsPublicKey,
        receipt: InactiveValidatorReceipt,
    ) -> Result<(), AccountError> {
        self.retire_validator(validator_key, receipt.retire_time)
    }

    /// Removes a validator from the parking lists.
    pub(super) fn unpark_validator(
        &mut self,
        validator_key: &BlsPublicKey,
    ) -> Result<UnparkReceipt, AccountError> {
        let current_epoch = self.current_epoch_parking.remove(validator_key);
        let previous_epoch = self.previous_epoch_parking.remove(validator_key);

        if !current_epoch && !previous_epoch {
            return Err(AccountError::InvalidForRecipient);
        }

        Ok(UnparkReceipt {
            current_epoch,
            previous_epoch,
        })
    }

    /// Reverts an unparking transaction.
    pub(super) fn revert_unpark_validator(
        &mut self,
        validator_key: &BlsPublicKey,
        receipt: UnparkReceipt,
    ) -> Result<(), AccountError> {
        if receipt.current_epoch {
            self.current_epoch_parking.insert(validator_key.clone());
        }

        if receipt.previous_epoch {
            self.previous_epoch_parking.insert(validator_key.clone());
        }

        Ok(())
    }
}
