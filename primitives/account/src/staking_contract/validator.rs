use std::cmp::min;

use log::error;

use beserial::{Deserialize, Serialize};
use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_database::WriteTransaction;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;

use crate::staking_contract::receipts::{
    DropValidatorReceipt, ReactivateValidatorReceipt, RetireValidatorReceipt,
    UnparkValidatorReceipt, UpdateValidatorReceipt,
};
use crate::{Account, AccountError, AccountsTrie, StakingContract};

/// Struct representing a validator in the staking contract.
/// Actions concerning a validator are:
/// 1. Create: Creates a validator.
/// 2. Update: Updates the validator.
/// 3. Retire: Inactivates a validator (also starts a cooldown period used for Drop).
/// 4. Reactivate: Reactivates a validator.
/// 5. Unpark: Prevents a validator from being automatically inactivated.
/// 6. Drop: Drops a validator (validator must have been inactive for the cooldown period).
///
/// The actions can be summarized by the following state diagram:
///        +--------+   retire    +----------+
/// create |        +------------>+          | drop
///+------>+ active |             | inactive +------>
///        |        +<------------+          |
///        +-+--+---+  reactivate +-----+----+
///          |  ^                       ^
///          |  |                       |
///          |  | unpark                | automatically
/// slashing |  |                       |
///          |  |     +--------+        |
///          |  +-----+        |        |
///          |        | parked +--------+
///          +------->+        |
///                   +--------+
///
/// Create, Update, Retire, Re-activate and Unpark are incoming transactions to the staking contract.
/// Drop is an outgoing transaction from the staking contract.
/// To Create, Update or Drop, the validator must use the cold key (the one corresponding to the
/// validator address). For the other transactions, the validator must use the warm key (the one
/// corresponding to the warm address).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validator {
    // The address of the validator. The corresponding key can be used to create, update or drop
    // the validator.
    pub address: Address,
    // This key is used to retire, reactivate and unpark the validator.
    pub warm_key: Address,
    // The validator key, it is only used to sign blocks.
    pub validator_key: BlsPublicKey,
    // The reward address of the validator. All the block rewards are paid to this address.
    pub reward_address: Address,
    // Signalling field. Can be used to do chain upgrades or for any other purpose that requires
    // validators to coordinate among themselves.
    pub signal_data: Option<Blake2bHash>,
    // The amount of coins held by this validator. It also includes the coins delegated to him by
    // stakers.
    pub balance: Coin,
    // The number of stakers that are staking for this validator.
    pub num_stakers: u64,
    // A flag stating if the validator is inactive. If it is inactive, then it contains the block
    // height at which it became inactive.
    pub inactivity_flag: Option<u32>,
}

impl StakingContract {
    /// Creates a new validator. The initial stake is always equal to the validator deposit
    /// and can only be retrieved by dropping the validator.
    /// This function is public to fill the genesis staking contract.
    pub fn create_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        warm_key: Address,
        validator_key: BlsPublicKey,
        reward_address: Address,
        signal_data: Option<Blake2bHash>,
    ) -> Result<(), AccountError> {
        // Get the deposit value.
        let deposit = Coin::from_u64_unchecked(policy::VALIDATOR_DEPOSIT);

        // See if the validator already exists.
        if StakingContract::get_validator(accounts_tree, db_txn, validator_address).is_some() {
            return Err(AccountError::AlreadyExistentAddress {
                address: validator_address.clone(),
            });
        }

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_add(staking_contract.balance, deposit)?;

        staking_contract
            .active_validators
            .insert(validator_address.clone(), deposit);

        // Create validator struct.
        let validator = Validator {
            address: validator_address.clone(),
            warm_key,
            validator_key,
            reward_address,
            signal_data,
            balance: deposit,
            num_stakers: 0,
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
            "Trying to put validator with address {} in the accounts tree.",
            validator_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(())
    }

    /// Reverts creating a new validator entry.
    pub(crate) fn revert_create_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
    ) -> Result<(), AccountError> {
        // Get the deposit value.
        let deposit = Coin::from_u64_unchecked(policy::VALIDATOR_DEPOSIT);

        // See if the validator does not exist.
        if StakingContract::get_validator(accounts_tree, db_txn, validator_address).is_none() {
            return Err(AccountError::NonExistentAddress {
                address: validator_address.clone(),
            });
        }

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.balance = Account::balance_sub(staking_contract.balance, deposit)?;

        staking_contract.active_validators.remove(validator_address);

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to remove validator with address {} in the accounts tree.",
            validator_address.to_string(),
        );

        accounts_tree.remove(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
        );

        Ok(())
    }

    /// Updates some of the validator details (warm key, validator key, reward address and/or signal data).
    pub(crate) fn update_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        new_warm_key: Option<Address>,
        new_validator_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
    ) -> Result<UpdateValidatorReceipt, AccountError> {
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

        // Create receipt now.
        let receipt = UpdateValidatorReceipt {
            old_warm_key: validator.warm_key.clone(),
            old_validator_key: validator.validator_key.clone(),
            old_reward_address: validator.reward_address.clone(),
            old_signal_data: validator.signal_data.clone(),
        };

        // Update validator info.
        if let Some(value) = new_warm_key {
            validator.warm_key = value;
        }

        if let Some(value) = new_validator_key {
            validator.validator_key = value;
        }

        if let Some(value) = new_reward_address {
            validator.reward_address = value;
        }

        if let Some(value) = new_signal_data {
            validator.signal_data = value;
        }

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put validator with address {} in the accounts tree.",
            validator_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(receipt)
    }

    /// Reverts updating validator details.
    pub(crate) fn revert_update_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        receipt: UpdateValidatorReceipt,
    ) -> Result<(), AccountError> {
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

        // Revert validator info.
        validator.warm_key = receipt.old_warm_key;
        validator.validator_key = receipt.old_validator_key;
        validator.reward_address = receipt.old_reward_address;
        validator.signal_data = receipt.old_signal_data;

        // All checks passed, not allowed to fail from here on!
        trace!(
            "Trying to put validator with address {} in the accounts tree.",
            validator_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(())
    }

    /// Inactivates a validator. It is necessary to retire a validator before dropping it. This also
    /// removes the validator from the parking set.
    pub(crate) fn retire_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        warm_address: Address,
        block_height: u32,
    ) -> Result<RetireValidatorReceipt, AccountError> {
        // Get the validator and update it.
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                Some(v) => v,
                None => {
                    return Err(AccountError::NonExistentAddress {
                        address: validator_address.clone(),
                    });
                }
            };

        if warm_address != validator.warm_key {
            error!(
                "The warm address that signed the transaction doesn't match the warm address of the validator."
            );
            return Err(AccountError::InvalidSignature);
        }

        validator.inactivity_flag = Some(block_height);

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        if staking_contract
            .active_validators
            .remove(validator_address)
            .is_none()
        {
            error!(
                "Tried to inactivate a validator that was already inactivated! It has address {}.",
                validator_address
            );
            return Err(AccountError::InvalidForRecipient);
        }

        let parked_set = staking_contract.parked_set.remove(validator_address);

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with address {} in the accounts tree.",
            validator_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(RetireValidatorReceipt { parked_set })
    }

    /// Reverts inactivating a validator.
    pub(crate) fn revert_retire_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        receipt: RetireValidatorReceipt,
    ) -> Result<(), AccountError> {
        // Get the validator and update it.
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                Some(v) => v,
                None => {
                    return Err(AccountError::NonExistentAddress {
                        address: validator_address.clone(),
                    });
                }
            };

        validator.inactivity_flag = None;

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract
            .active_validators
            .insert(validator_address.clone(), validator.balance);

        if receipt.parked_set {
            staking_contract
                .parked_set
                .insert(validator_address.clone());
        }

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with address {} in the accounts tree.",
            validator_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(())
    }

    /// Reactivates a validator.
    pub(crate) fn reactivate_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        warm_address: Address,
    ) -> Result<ReactivateValidatorReceipt, AccountError> {
        // Get the validator and check that the signature is valid.
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                Some(v) => v,
                None => {
                    return Err(AccountError::NonExistentAddress {
                        address: validator_address.clone(),
                    });
                }
            };

        if warm_address != validator.warm_key {
            error!(
                "The warm address that signed the transaction doesn't match the warm address of the validator."
            );
            return Err(AccountError::InvalidSignature);
        }

        // Create receipt now.
        let receipt = match validator.inactivity_flag {
            Some(block_height) => ReactivateValidatorReceipt {
                retire_time: block_height,
            },
            None => {
                error!(
                    "Tried to re-activate a validator that was already active! It has address {}.",
                    validator_address
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
            .insert(validator_address.clone(), validator.balance);

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with address {} in the accounts tree.",
            validator_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(receipt)
    }

    /// Reverts reactivating a validator.
    pub(crate) fn revert_reactivate_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        receipt: ReactivateValidatorReceipt,
    ) -> Result<(), AccountError> {
        // Get the validator and update it.
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                Some(v) => v,
                None => {
                    return Err(AccountError::NonExistentAddress {
                        address: validator_address.clone(),
                    });
                }
            };

        validator.inactivity_flag = Some(receipt.retire_time);

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        staking_contract.active_validators.remove(validator_address);

        // All checks passed, not allowed to fail from here on!
        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        trace!(
            "Trying to put validator with address {} in the accounts tree.",
            validator_address.to_string(),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(())
    }

    /// Removes a validator from the parked set and the disabled slots. This is used by validators
    /// after they get slashed so that they can produce blocks again.
    pub(crate) fn unpark_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        warm_address: Address,
    ) -> Result<UnparkValidatorReceipt, AccountError> {
        // Get the validator and check that the signature is valid.
        let validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                Some(v) => v,
                None => {
                    return Err(AccountError::NonExistentAddress {
                        address: validator_address.clone(),
                    });
                }
            };

        if warm_address != validator.warm_key {
            error!(
                "The warm address that signed the transaction doesn't match the warm address of the validator."
            );
            return Err(AccountError::InvalidSignature);
        }

        // Get the staking contract and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let parked_set = staking_contract.parked_set.remove(validator_address);

        let current_disabled = staking_contract
            .current_disabled_slots
            .remove(validator_address);

        let previous_disabled = staking_contract
            .previous_disabled_slots
            .remove(validator_address);

        if !parked_set && current_disabled.is_none() && previous_disabled.is_none() {
            error!(
                "Tried to unpark a validator that was already unparked! It has address {}.",
                validator_address
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
            parked_set,
            current_disabled_slots: current_disabled,
            previous_disabled_slots: previous_disabled,
        })
    }

    /// Reverts an unparking transaction.
    pub(crate) fn revert_unpark_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        receipt: UnparkValidatorReceipt,
    ) -> Result<(), AccountError> {
        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        if receipt.parked_set {
            staking_contract
                .parked_set
                .insert(validator_address.clone());
        }

        if let Some(slots) = receipt.current_disabled_slots {
            staking_contract
                .current_disabled_slots
                .insert(validator_address.clone(), slots);
        }

        if let Some(slots) = receipt.previous_disabled_slots {
            staking_contract
                .previous_disabled_slots
                .insert(validator_address.clone(), slots);
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

    /// Drops a validator and returns its deposit. This can only be used on inactive validators!
    /// The validator must have been inactive for at least one election block.
    pub(crate) fn drop_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        block_height: u32,
    ) -> Result<DropValidatorReceipt, AccountError> {
        // Get the validator.
        let validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                Some(v) => v,
                None => {
                    return Err(AccountError::NonExistentAddress {
                        address: validator_address.clone(),
                    });
                }
            };

        // Check that the validator has been inactive for long enough.
        match validator.inactivity_flag {
            None => {
                error!(
                    "Tried to drop a validator which was still active! Validator address {}",
                    validator_address
                );
                return Err(AccountError::InvalidForSender);
            }
            Some(time) => {
                if block_height <= policy::election_block_after(time) {
                    return Err(AccountError::InvalidForSender);
                }
            }
        }

        // All checks passed, not allowed to fail from here on!

        // Initialize the receipts.
        let mut receipt = DropValidatorReceipt {
            warm_key: validator.warm_key,
            validator_key: validator.validator_key,
            reward_address: validator.reward_address,
            signal_data: validator.signal_data,
            retire_time: validator.inactivity_flag.expect(
                "This can't fail since we already checked above that the inactivity flag is Some.",
            ),
            stakers: vec![],
        };

        // Remove the validator from all its stakers. Also delete all the validator's stakers entries.
        let empty_staker_key =
            StakingContract::get_key_validator_staker(validator_address, &Address::from([0; 20]));

        let mut remaining_stakers = validator.num_stakers as usize;

        // Here we use a chunk size of 100. It's completely arbitrary, we just don't want to
        // download the entire staker list into memory since it might be huge.
        let chunk_size = 100;

        while remaining_stakers > 0 {
            // Get chunk of stakers.
            let chunk = accounts_tree.get_chunk(
                db_txn,
                &empty_staker_key,
                min(remaining_stakers, chunk_size),
            );

            // Update the number of stakers.
            remaining_stakers -= chunk.len();

            for account in chunk {
                if let Account::StakingValidatorsStaker(staker_address) = account {
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
                        &StakingContract::get_key_validator_staker(
                            validator_address,
                            &staker_address,
                        ),
                    );

                    // Update the receipt.
                    receipt.stakers.push(staker_address);
                } else {
                    panic!("When trying to fetch a staker for a validator we got a different type of account. This should never happen!");
                }
            }
        }

        // Remove the validator entry.
        accounts_tree.remove(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
        );

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let deposit = Coin::from_u64_unchecked(policy::VALIDATOR_DEPOSIT);

        staking_contract.balance = Account::balance_sub(staking_contract.balance, deposit)?;

        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        // Return the receipt.
        Ok(receipt)
    }

    /// Reverts dropping a validator.
    pub(crate) fn revert_drop_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
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
            balance += u64::from(staker.active_stake);

            // Update the staker.
            staker.delegation = Some(validator_address.clone());

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
                &StakingContract::get_key_validator_staker(validator_address, &staker_address),
                Account::StakingValidatorsStaker(staker_address.clone()),
            );
        }

        // Re-add the validator entry.
        let validator = Validator {
            address: validator_address.clone(),
            warm_key: receipt.warm_key,
            validator_key: receipt.validator_key,
            reward_address: receipt.reward_address,
            signal_data: receipt.signal_data,
            balance: Coin::from_u64_unchecked(balance + policy::VALIDATOR_DEPOSIT),
            num_stakers,
            inactivity_flag: Some(receipt.retire_time),
        };

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let deposit = Coin::from_u64_unchecked(policy::VALIDATOR_DEPOSIT);

        staking_contract.balance = Account::balance_add(staking_contract.balance, deposit)?;

        trace!("Trying to put staking contract in the accounts tree.");

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(())
    }
}
