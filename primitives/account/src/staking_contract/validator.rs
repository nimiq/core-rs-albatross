use log::error;
use std::cmp::{min, Ordering};
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
use nimiq_primitives::policy;

use crate::staking_contract::receipts::{
    DropValidatorReceipt, ReactivateValidatorOrStakerReceipt, RetireValidatorReceipt,
    RetirementReceipt, UnparkValidatorReceipt, UpdateValidatorReceipt,
};
use crate::{Account, AccountError, AccountsTree, StakingContract};
use nimiq_hash::Blake2bHash;
use nimiq_trie::key_nibbles::KeyNibbles;

/// Struct representing a validator in the staking contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Validator {
    // The id of the validator, the first 20 bytes of the transaction hash from which it was created.
    pub id: ValidatorId,
    // The amount of coins held by this validator. It also includes the coins delegated to him by
    // stakers.
    pub balance: Coin,
    // The number of stakers that are staking for this validator.
    pub num_stakers: u64,
    // The reward address of the validator.
    pub reward_address: Address,
    // The validator key.
    pub validator_key: BlsPublicKey,
    // Arbitrary data field. Can be used to do chain upgrades or for any other purpose that requires
    // validators to communicate arbitrary data.
    pub extra_data: Option<Blake2bHash>,
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
    /// Creates a new validator entry. The initial stake is always equal to the minimum validator
    /// stake and can only be retrieved by dropping the validator.
    /// This is public to fill the genesis staking contract.
    pub fn create_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        reward_address: Address,
        validator_key: BlsPublicKey,
        extra_data: Option<Blake2bHash>,
    ) -> Result<(), AccountError> {
        // Get the initial stake value.
        let initial_stake = Coin::from_u64_unchecked(policy::MIN_VALIDATOR_STAKE);

        // See if the validator already exists.
        if StakingContract::get_validator(accounts_tree, db_txn, validator_id).is_some() {
            return Err(AccountError::AlreadyExistentValidator {
                id: validator_id.clone(),
            });
        }

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_add(staking_contract.balance, initial_stake)?;

        staking_contract
            .active_validators
            .insert(validator_id.clone(), initial_stake);

        // Create validator struct.
        let validator = Validator {
            id: validator_id.clone(),
            balance: initial_stake,
            num_stakers: 0,
            reward_address,
            validator_key,
            extra_data,
            inactivity_flag: None,
        };

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with id {} in the accounts tree.",
            validator_id.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_id),
            Account::StakingValidator(validator),
        );

        Ok(())
    }

    /// Reverts creating a new validator entry.
    fn revert_create_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
    ) -> Result<(), AccountError> {
        // Get the initial stake value.
        let initial_stake = Coin::from_u64_unchecked(policy::MIN_VALIDATOR_STAKE);

        // See if the validator does not exists.
        if StakingContract::get_validator(accounts_tree, db_txn, validator_id).is_none() {
            return Err(AccountError::NonExistentValidator {
                id: validator_id.clone(),
            });
        }

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_sub(staking_contract.balance, initial_stake)?;

        staking_contract.active_validators.remove(validator_id);

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to remove validator with id {} in the accounts tree.",
            validator_id.to_string(),
        );

        accounts_tree.remove(db_txn, &StakingContract::get_key_validator(validator_id));

        Ok(())
    }

    /// Update validator details.
    fn update_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        new_validator_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
        new_extra_data: Option<Option<Blake2bHash>>,
    ) -> Result<UpdateValidatorReceipt, AccountError> {
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

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put validator with id {} in the accounts tree.",
            validator_id.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_id),
            Account::StakingValidator(validator),
        );

        Ok(receipt)
    }

    /// Reverts updating validator details.
    fn revert_update_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        receipt: UpdateValidatorReceipt,
    ) -> Result<(), AccountError> {
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

        // Revert validator info.
        validator.validator_key = receipt.old_validator_key;
        validator.reward_address = receipt.old_reward_address;
        validator.extra_data = receipt.old_extra_data;

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put validator with id {} in the accounts tree.",
            validator_id.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_id),
            Account::StakingValidator(validator),
        );

        Ok(())
    }

    /// Inactivates a validator. This also removes the validator from the parking sets.
    fn retire_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        block_height: u32,
    ) -> Result<RetireValidatorReceipt, AccountError> {
        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        if staking_contract
            .active_validators
            .remove(validator_id)
            .is_none()
        {
            error!(
                "Tried to inactivate a validator that was already inactivated! It has id {}.",
                validator_id
            );
            return Err(AccountError::InvalidForRecipient);
        }

        let current_parking = staking_contract.current_epoch_parking.remove(validator_id);
        let previous_parking = staking_contract.previous_epoch_parking.remove(validator_id);

        // Get the validator and update it.
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_id) {
                Some(v) => v,
                None => {
                    return Err(AccountError::NonExistentValidator {
                        id: validator_id.clone(),
                    });
                }
            };

        validator.inactivity_flag = Some(block_height);

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with id {} in the accounts tree.",
            validator_id.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_id),
            Account::StakingValidator(validator),
        );

        Ok(RetireValidatorReceipt {
            current_epoch_parking,
            previous_epoch_parking,
        })
    }

    /// Reverts inactivating a validator entry.
    fn revert_retire_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        receipt: RetireValidatorReceipt,
    ) -> Result<(), AccountError> {
        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        if receipt.current_epoch_parking {
            staking_contract
                .current_epoch_parking
                .insert(validator_id.clone());
        }

        if receipt.previous_epoch_parking {
            staking_contract
                .previous_epoch_parking
                .insert(validator_id.clone());
        }

        // Get the validator and update it.
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_id) {
                Some(v) => v,
                None => {
                    return Err(AccountError::NonExistentValidator {
                        id: validator_id.clone(),
                    });
                }
            };

        validator.inactivity_flag = Some(block_height);

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with id {} in the accounts tree.",
            validator_id.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_id),
            Account::StakingValidator(validator),
        );

        Ok(())
    }

    /// Reactivate a validator entry.
    fn reactivate_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
    ) -> Result<ReactivateValidatorOrStakerReceipt, AccountError> {
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

        // Create receipt now.
        let receipt = match validator.inactivity_flag {
            Some(block_height) => ReactivateValidatorOrStakerReceipt {
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

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with id {} in the accounts tree.",
            validator_id.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_id),
            Account::StakingValidator(validator),
        );

        Ok(receipt)
    }

    /// Reverts re-activating a validator entry.
    fn revert_reactivate_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        receipt: ReactivateValidatorOrStakerReceipt,
    ) -> Result<(), AccountError> {
        // Get the validator and update it.
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_id) {
                Some(v) => v,
                None => {
                    return Err(AccountError::NonExistentValidator {
                        id: validator_id.clone(),
                    });
                }
            };

        validator.inactivity_flag = Some(receipt.retire_time);

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.active_validators.remove(validator_id);

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with id {} in the accounts tree.",
            validator_id.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_id),
            Account::StakingValidator(validator),
        );

        Ok(())
    }

    /// Removes a validator from the parking lists and the disabled slots.
    fn unpark_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
    ) -> Result<UnparkValidatorReceipt, AccountError> {
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
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(UnparkValidatorReceipt {
            current_epoch_parking: current_epoch,
            previous_epoch_parking: previous_epoch,
            current_disabled_slots,
        })
    }

    /// Reverts an unparking transaction.
    fn revert_unpark_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        receipt: UnparkValidatorReceipt,
    ) -> Result<(), AccountError> {
        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        if receipt.current_epoch_parking {
            staking_contract
                .current_epoch_parking
                .insert(validator_id.clone());
        }

        if receipt.previous_epoch_parking {
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
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(())
    }

    /// Drops a validator entry. This can only be used to drop inactive validators!
    /// The validator must have been inactive for at least one election block.
    fn drop_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        block_height: u32,
    ) -> Result<DropValidatorReceipt, AccountError> {
        // Get the validator.
        let validator = match StakingContract::get_validator(accounts_tree, db_txn, validator_id) {
            Some(v) => v,
            None => {
                return Err(AccountError::NonExistentValidator {
                    id: validator_id.clone(),
                });
            }
        };

        // Check that the validator has been inactive for long enough.
        match validator.inactivity_flag {
            None => {
                error!(
                    "Tried to drop a validator which was still active! Validator ID {}",
                    validator_id
                );
                return Err(AccountError::InvalidForSender);
            }
            Some(time) => {
                if block_height < policy::election_block_after(time) {
                    return Err(AccountError::InvalidForSender);
                }
            }
        }

        // All checks passed, not allowed to fail from here on!

        // Initialize the receipts.
        let mut receipt = DropValidatorReceipt {
            reward_address: validator.reward_address,
            validator_key: validator.validator_key,
            extra_data: validator.extra_data,
            retire_time: validator.inactivity_flag.expect(
                "This can't fail since we already checked above that the inactivity flag is Some.",
            ),
            stakers: vec![],
        };

        // Remove the validator from all its stakers. Also delete all the validator's stakers entries.
        let empty_staker_key =
            StakingContract::get_key_validator_staker(validator_id, &Address::from([0; 20]));

        let mut remaining_stakers = validator.num_stakers as usize;

        // Here we use a chunk size of 100. It's completely arbitrary, we just don't want to
        // download the entire staker list into memory since it might be huge.
        let chunk_size = 100;

        while remaining_stakers > 0 {
            // Get chunk of stakers.
            let chunk =
                accounts_tree.get_chunk(txn, &empty_staker_key, min(remaining_stakers, chunk_size));

            // Update the number of stakers.
            remaining_stakers -= chunk.len();

            for account in chunk {
                if let Account::StakingValidatorStaker(staker_address) = account {
                    // Update the staker.
                    let mut staker = StakingContract::get_staker(accounts_tree, db_txn, &staker_address).expect("A validator had an staker staking for it that doesn't exist in the Accounts Tree!");

                    staker.delegation = None;

                    trace!(
                        "Trying to put staker with address {} in the accounts tree.",
                        staker_address.to_string(),
                    );

                    accounts_tree.put(
                        db_txn,
                        &StakingContract::get_key_staker(&staker_address),
                        Account::StakingStaker(staker),
                    );

                    // Remove the staker entry from the validator.
                    trace!(
                        "Trying to remove validator's staker with address {} in the accounts tree.",
                        staker_address.to_string(),
                    );

                    accounts_tree.remove(
                        db_txn,
                        &StakingContract::get_key_validator_staker(validator_id, &staker_address),
                    );

                    // Update the receipt.
                    receipt.stakers.push(staker_address);
                } else {
                    panic!("When trying to fetch a staker for a validator we got a different type of account. This should never happen!");
                }
            }
        }

        // Remove the validator entry.
        accounts_tree.remove(db_txn, &StakingContract::get_key_validator(validator_id));

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let initial_stake = Coin::from_u64_unchecked(policy::MIN_VALIDATOR_STAKE);

        staking_contract.balance = Account::balance_sub(staking_contract.balance, initial_stake)?;

        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        // Return the receipt.
        Ok(receipt)
    }

    /// Revert dropping a validator entry.
    fn revert_drop_validator(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        validator_id: &ValidatorId,
        receipt: DropValidatorReceipt,
    ) -> Result<(), AccountError> {
        // Re-add the validator to all its stakers. Also create all the validator's stakers entries.
        let mut num_stakers = 0;

        let mut balance = 0;

        for staker_address in receipt.stakers {
            // Get the staker.
            let mut staker = StakingContract::get_staker(accounts_tree, db_txn, &staker_address)
                .expect(
                "A validator had an staker staking for it that doesn't exist in the Accounts Tree!",
            );

            // Update the counters.
            num_stakers += 1;
            balance += u64::from(staker.balance);

            // Update the staker.
            staker.delegation = Some(validator_id.clone());

            trace!(
                "Trying to put staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_staker(&staker_address),
                Account::StakingStaker(staker),
            );

            // Add the staker entry to the validator.
            trace!(
                "Trying to put validator's staker with address {} in the accounts tree.",
                staker_address.to_string(),
            );

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(validator_id, &staker_address),
                Account::StakingValidatorStaker(staker_address.clone()),
            );
        }

        // Re-add the validator entry.
        let validator = Validator {
            id: validator_id.clone(),
            balance: Coin::from_u64_unchecked(balance + policy::MIN_VALIDATOR_STAKE),
            num_stakers,
            reward_address: receipt.reward_address,
            validator_key: receipt.validator_key,
            extra_data: receipt.extra_data,
            inactivity_flag: Some(receipt.retire_time),
        };

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_id),
            Account::StakingValidator(validator),
        );

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let initial_stake = Coin::from_u64_unchecked(policy::MIN_VALIDATOR_STAKE);

        staking_contract.balance = Account::balance_sub(staking_contract.balance, initial_stake)?;

        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(())
    }
}
