use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use parking_lot::RwLock;

use beserial::{
    Deserialize, DeserializeWithLength, ReadBytesExt, Serialize, SerializeWithLength,
    SerializingError, WriteBytesExt,
};
use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_database::{Database, Environment, Transaction as DBTransaction, WriteTransaction};
use nimiq_keys::Address;
use nimiq_primitives::account::ValidatorId;
use nimiq_primitives::coin::Coin;

use crate::staking_contract::receipts::{
    DropValidatorReceipt, InactiveValidatorReceipt, RetirementReceipt, UnparkReceipt,
    UpdateValidatorReceipt,
};
use crate::{Account, AccountError, AccountsTree, StakingContract};
use nimiq_trie::key_nibbles::KeyNibbles;

/// Struct representing a validator in the staking contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Validator {
    // The id of the validator, the first 20 bytes of the transaction hash from which it was created.
    pub id: ValidatorId,
    // The amount of coins held by this validator. It also includes the coins delegated to him by
    // stakers.
    pub balance: Coin,
    // The reward address of the validator.
    pub reward_address: Address,
    // The validator key.
    pub validator_key: BlsPublicKey,
    // Arbitrary data field. Can be used to do chain upgrades or for any other purpose that requires
    // validators to communicate arbitrary data.
    pub extra_data: Option<[u8; 64]>,
    // A flag stating if the validator is inactive. If it is inactive, then it contains the block
    // height at which it became inactive.
    pub inactive_flag: Option<u32>,
}

/// Actions concerning a validator are:
/// 1. Create: Creates a validator entry.
/// 2. Update: Updates the validator entry.
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
    /// This is public to fill the genesis staking contract.
    pub fn create_validator(
        &mut self,
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: ValidatorId,
        balance: Coin,
        reward_address: Address,
        validator_key: BlsPublicKey,
        extra_data: Option<[u8; 64]>,
    ) -> Result<(), AccountError> {
        if StakingContract::get_validator(accounts_tree, db_txn, &validator_id).is_some() {
            return Err(AccountError::AlreadyExistentValidator { id: validator_id });
        }

        self.balance = Account::balance_add(self.balance, balance)?;

        // All checks passed, not allowed to fail from here on!
        let validator = Validator {
            id: validator_id.clone(),
            balance,
            reward_address,
            validator_key,
            extra_data,
            inactive_flag: None,
        };

        self.active_validators.insert(validator_id, balance);

        // Calculate key for the validator in the accounts tree.
        let mut bytes: Vec<u8> = Vec::with_capacity(41);
        bytes.extend(StakingContract::ADDRESS.as_bytes());
        // This is the byte flag for the validator list in the staking contract.
        bytes.push(1u8);
        bytes.extend(validator_id.as_bytes());

        let key = KeyNibbles::from(&bytes);

        trace!(
            "Trying to put validator with id {} at key {}.",
            validator_id.to_string(),
            key.to_string()
        );

        accounts_tree.put(db_txn, &key, Account::StakingValidator(validator));

        Ok(())
    }

    /// Reverts creating a new validator entry.
    pub(super) fn revert_create_validator(
        &mut self,
        validator_id: ValidatorId,
        _validator_key: BlsPublicKey,
        initial_stake: Coin,
    ) -> Result<(), AccountError> {
        if let Some(validator) = self.active_validators_by_id.remove(&validator_id) {
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
        validator_id: &ValidatorId,
        new_validator_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
    ) -> Result<UpdateValidatorReceipt, AccountError> {
        let mut entry = self
            .remove_validator(validator_id)
            .ok_or(AccountError::InvalidForRecipient)?;

        let old_reward_address = entry.as_validator().reward_address.clone();
        entry.update_validator(new_reward_address, new_validator_key);
        self.restore_validator(entry)?;

        Ok(UpdateValidatorReceipt { old_reward_address })
    }

    /// Reverts updating validator key.
    pub(super) fn revert_update_validator(
        &mut self,
        validator_id: ValidatorId,
        old_validator_key: BlsPublicKey,
        receipt: UpdateValidatorReceipt,
    ) -> Result<(), AccountError> {
        let mut entry = self
            .remove_validator(&validator_id)
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
        validator_id: &ValidatorId,
        _initial_stake: Coin,
    ) -> Result<DropValidatorReceipt, AccountError> {
        // Initial stake vs. stakes has been checked in check_outgoing_transaction.

        // All checks passed, not allowed to fail from here on!
        // Retire all stakes.
        let inactive_validator = self.inactive_validators_by_id.remove(validator_id).unwrap();
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
        let unpark_receipt = if self.current_epoch_parking.contains(validator_id)
            || self.previous_epoch_parking.contains(validator_id)
            || self.current_disabled_slots.contains_key(validator_id)
            || self.previous_disabled_slots.contains_key(validator_id)
        {
            self.unpark_validator(validator_id)?
        } else {
            UnparkReceipt {
                current_epoch: false,
                previous_epoch: false,
                current_disabled_slots: None,
                previous_disabled_slots: None,
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
        validator_id: ValidatorId,
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
        self.revert_unpark_validator(&validator_id, receipt.unpark_receipt)?;

        self.inactive_validators_by_id.insert(
            validator_id.clone(),
            InactiveValidator {
                validator: Arc::new(Validator {
                    id: validator_id,
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
        validator_id: ValidatorId,
        block_height: u32,
    ) -> Result<(), AccountError> {
        // Move validator from active map/set to inactive map.
        let validator = self
            .active_validators_by_id
            .remove(&validator_id)
            .ok_or(AccountError::InvalidForRecipient)?;

        // All checks passed, not allowed to fail from here on!
        self.active_validators_sorted.remove(&validator);
        self.inactive_validators_by_id.insert(
            validator_id,
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
        validator_id: ValidatorId,
    ) -> Result<(), AccountError> {
        self.reactivate_validator(validator_id).map(|_| ())
    }

    /// Reactivate a validator entry.
    pub(super) fn reactivate_validator(
        &mut self,
        validator_id: ValidatorId,
    ) -> Result<InactiveValidatorReceipt, AccountError> {
        // Move validator from inactive map to active map/set.
        let inactive_validator = self
            .inactive_validators_by_id
            .remove(&validator_id)
            .ok_or(AccountError::InvalidForRecipient)?;

        // All checks passed, not allowed to fail from here on!
        self.active_validators_sorted
            .insert(Arc::clone(&inactive_validator.validator));
        self.active_validators_by_id
            .insert(validator_id, inactive_validator.validator);
        Ok(InactiveValidatorReceipt {
            retire_time: inactive_validator.retire_time,
        })
    }

    /// Inactivates a validator entry.
    pub(super) fn revert_reactivate_validator(
        &mut self,
        validator_id: ValidatorId,
        receipt: InactiveValidatorReceipt,
    ) -> Result<(), AccountError> {
        self.retire_validator(validator_id, receipt.retire_time)
    }

    /// Removes a validator from the parking lists and the disabled slots.
    pub(super) fn unpark_validator(
        &mut self,
        validator_id: &ValidatorId,
    ) -> Result<UnparkReceipt, AccountError> {
        let current_epoch = self.current_epoch_parking.remove(validator_id);
        let previous_epoch = self.previous_epoch_parking.remove(validator_id);

        let current_disabled_slots = self.current_disabled_slots.remove(validator_id);
        let previous_disabled_slots = self.previous_disabled_slots.remove(validator_id);

        if !current_epoch
            && !previous_epoch
            && current_disabled_slots.is_none()
            && previous_disabled_slots.is_none()
        {
            return Err(AccountError::InvalidForRecipient);
        }

        Ok(UnparkReceipt {
            current_epoch,
            previous_epoch,
            current_disabled_slots,
            previous_disabled_slots,
        })
    }

    /// Reverts an unparking transaction.
    pub(super) fn revert_unpark_validator(
        &mut self,
        validator_id: &ValidatorId,
        receipt: UnparkReceipt,
    ) -> Result<(), AccountError> {
        if receipt.current_epoch {
            self.current_epoch_parking.insert(validator_id.clone());
        }

        if receipt.previous_epoch {
            self.previous_epoch_parking.insert(validator_id.clone());
        }

        if let Some(slots) = receipt.current_disabled_slots {
            self.current_disabled_slots
                .insert(validator_id.clone(), slots);
        }

        if let Some(slots) = receipt.previous_disabled_slots {
            self.previous_disabled_slots
                .insert(validator_id.clone(), slots);
        }

        Ok(())
    }
}
