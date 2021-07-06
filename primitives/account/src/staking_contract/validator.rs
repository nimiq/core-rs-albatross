use log::error;
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
    pub extra_data: Option<[u8; 32]>,
    // A flag stating if the validator is inactive. If it is inactive, then it contains the block
    // height at which it became inactive.
    pub inactivity_flag: Option<u32>,
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
    /// Creates a new validator entry. The initial stake can only be retrieved by dropping the
    /// validator. This is public to fill the genesis staking contract.
    pub fn create_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: ValidatorId,
        initial_stake: Coin,
        reward_address: Address,
        validator_key: BlsPublicKey,
        extra_data: Option<[u8; 32]>,
    ) -> Result<(), AccountError> {
        // See if the validator already exists.
        if StakingContract::get_validator(accounts_tree, db_txn, &validator_id).is_some() {
            return Err(AccountError::AlreadyExistentValidator { id: validator_id });
        }

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_add(staking_contract.balance, initial_stake)?;

        staking_contract
            .active_validators
            .insert(validator_id, initial_stake);

        // Calculate key for the validator in the accounts tree.
        let key = StakingContract::get_key_validator(&validator_id);

        // Create validator struct.
        let validator = Validator {
            id: validator_id.clone(),
            balance: initial_stake,
            reward_address,
            validator_key,
            extra_data,
            inactivity_flag: None,
        };

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put staking contract with at key {}.",
            StakingContract::get_key_staking_contract().to_string()
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with id {} at key {}.",
            validator_id.to_string(),
            key.to_string()
        );

        accounts_tree.put(db_txn, &key, Account::StakingValidator(validator));

        Ok(())
    }

    /// Reverts creating a new validator entry.
    fn revert_create_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: ValidatorId,
        initial_stake: Coin,
    ) -> Result<(), AccountError> {
        // See if the validator does not exists.
        if StakingContract::get_validator(accounts_tree, db_txn, &validator_id).is_none() {
            return Err(AccountError::NonExistentValidator { id: validator_id });
        }

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_sub(staking_contract.balance, initial_stake)?;

        staking_contract.active_validators.remove(&validator_id);

        // Calculate key for the validator in the accounts tree.
        let key = StakingContract::get_key_validator(&validator_id);

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put staking contract with at key {}.",
            StakingContract::get_key_staking_contract().to_string()
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to remove validator with id {} at key {}.",
            validator_id.to_string(),
            key.to_string()
        );

        accounts_tree.remove(db_txn, &key);

        Ok(())
    }

    /// Update validator details.
    fn update_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        new_validator_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
        new_extra_data: Option<Option<[u8; 32]>>,
    ) -> Result<UpdateValidatorReceipt, AccountError> {
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

        // Create receipt now.
        let receipt = UpdateValidatorReceipt {
            old_validator_key: validator.validator_key.clone(),
            old_reward_address: validator.reward_address.clone(),
            old_extra_data: validator.extra_data.clone(),
        };

        // Update validator info.
        if let Some(value) = new_validator_key {
            validator.validator_key = value;
        }

        if let Some(value) = new_reward_address {
            validator.reward_address = value;
        }

        if let Some(value) = new_extra_data {
            validator.extra_data = value;
        }

        // Calculate key for the validator in the accounts tree.
        let key = StakingContract::get_key_validator(&validator_id);

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put validator with id {} at key {}.",
            validator_id.to_string(),
            key.to_string()
        );

        accounts_tree.put(db_txn, &key, Account::StakingValidator(validator));

        Ok(receipt)
    }

    /// Reverts updating validator key.
    fn revert_update_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: ValidatorId,
        receipt: UpdateValidatorReceipt,
    ) -> Result<(), AccountError> {
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, &validator_id) {
                Some(v) => v,
                None => {
                    return Err(AccountError::NonExistentValidator {
                        id: validator_id.clone(),
                    });
                }
            };

        // Revert validator info.
        validator.validator_key = receipt.old_validator_key;
        validator.reward_address = receipt.old_reward_address;
        validator.extra_data = receipt.old_extra_data;

        // Calculate key for the validator in the accounts tree.
        let key = StakingContract::get_key_validator(&validator_id);

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put validator with id {} at key {}.",
            validator_id.to_string(),
            key.to_string()
        );

        accounts_tree.put(db_txn, &key, Account::StakingValidator(validator));

        Ok(())
    }

    /// Inactivates a validator.
    fn retire_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: ValidatorId,
        block_height: u32,
    ) -> Result<(), AccountError> {
        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        if staking_contract
            .active_validators
            .remove(&validator_id)
            .is_none()
        {
            error!(
                "Tried to inactivate a validator that was already inactivated! It has id {}.",
                validator_id
            );
            return Err(AccountError::InvalidForRecipient);
        }

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

        // Update validator inactivity flag.
        validator.inactivity_flag = Some(block_height);

        // Calculate key for the validator in the accounts tree.
        let key = StakingContract::get_key_validator(&validator_id);

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put staking contract with at key {}.",
            StakingContract::get_key_staking_contract().to_string()
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with id {} at key {}.",
            validator_id.to_string(),
            key.to_string()
        );

        accounts_tree.put(db_txn, &key, Account::StakingValidator(validator));

        Ok(())
    }

    /// Reverts inactivating a validator entry.
    fn revert_retire_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: ValidatorId,
    ) -> Result<(), AccountError> {
        StakingContract::reactivate_validator(accounts_tree, db_txn, validator_id).map(|_| ())
    }

    /// Reactivate a validator entry.
    fn reactivate_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: ValidatorId,
    ) -> Result<InactiveValidatorReceipt, AccountError> {
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

        // Create receipt now.
        let receipt = match validator.inactivity_flag {
            Some(block_height) => InactiveValidatorReceipt {
                retire_time: block_height,
            },
            None => {
                error!(
                    "Tried to re-activate a validator that was already active! It has id {}.",
                    validator_id
                );
                return Err(AccountError::InvalidForRecipient);
            }
        };

        // Update validator inactivity flag.
        validator.inactivity_flag = None;

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract
            .active_validators
            .insert(validator_id.clone(), validator.balance);

        // Calculate key for the validator in the accounts tree.
        let key = StakingContract::get_key_validator(&validator_id);

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put staking contract with at key {}.",
            StakingContract::get_key_staking_contract().to_string()
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with id {} at key {}.",
            validator_id.to_string(),
            key.to_string()
        );

        accounts_tree.put(db_txn, &key, Account::StakingValidator(validator));

        Ok(receipt)
    }

    /// Reverts re-activating a validator entry.
    fn revert_reactivate_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: ValidatorId,
        receipt: InactiveValidatorReceipt,
    ) -> Result<(), AccountError> {
        StakingContract::retire_validator(accounts_tree, db_txn, validator_id, receipt.retire_time)
    }

    /// Removes a validator from the parking lists and the disabled slots.
    fn unpark_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
    ) -> Result<UnparkReceipt, AccountError> {
        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let current_parking = staking_contract.current_epoch_parking.remove(validator_id);
        let previous_parking = staking_contract.previous_epoch_parking.remove(validator_id);
        let current_disabled = staking_contract.current_disabled_slots.remove(validator_id);

        if !current_parking && !previous_parking && current_disabled.is_none() {
            error!(
                "Tried to unpark a validator that was already unparked! It has id {}.",
                validator_id
            );
            return Err(AccountError::InvalidForRecipient);
        }

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put staking contract with at key {}.",
            StakingContract::get_key_staking_contract().to_string()
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(UnparkReceipt {
            current_epoch,
            previous_epoch,
            current_disabled_slots,
        })
    }

    /// Reverts an unparking transaction.
    fn revert_unpark_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        receipt: UnparkReceipt,
    ) -> Result<(), AccountError> {
        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        if receipt.current_epoch {
            staking_contract
                .current_epoch_parking
                .insert(validator_id.clone());
        }

        if receipt.previous_epoch {
            staking_contract
                .previous_epoch_parking
                .insert(validator_id.clone());
        }

        if let Some(slots) = receipt.current_disabled_slots {
            staking_contract
                .current_disabled_slots
                .insert(validator_id.clone(), slots);
        }

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put staking contract with at key {}.",
            StakingContract::get_key_staking_contract().to_string()
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(())
    }

    /// Drops a validator entry.
    /// This can be used to drop inactive validators.
    /// The validator must have been inactive for at least one macro block.
    fn drop_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
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
    fn revert_drop_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: ValidatorId,
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
}
