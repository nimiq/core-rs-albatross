use std::cmp::min;

use log::error;

use beserial::{Deserialize, Serialize};
use nimiq_bls::{CompressedPublicKey as BlsPublicKey, CompressedPublicKey};
use nimiq_database::WriteTransaction;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_primitives::{coin::Coin, policy::Policy};

use crate::logs::{Log, OperationInfo};
use crate::staking_contract::receipts::{
    DeleteValidatorReceipt, InactivateValidatorReceipt, ReactivateValidatorReceipt,
    UnparkValidatorReceipt, UpdateValidatorReceipt,
};
use crate::{Account, AccountError, AccountsTrie, Receipt, StakingContract};

/// Struct representing a validator in the staking contract.
/// Actions concerning a validator are:
/// 1. Create: Creates a validator.
/// 2. Update: Updates the validator.
/// 3. Inactivate: Inactivates a validator (also starts a cooldown period used for Delete).
/// 4. Reactivate: Reactivates a validator.
/// 5. Unpark: Prevents a validator from being automatically inactivated.
/// 6. Delete: Deletes a validator (validator must have been inactive for the cooldown period).
///
/// The actions can be summarized by the following state diagram:
///        +--------+          retire           +----------+
/// create |        +-------------------------->+          | drop
///+------>+ active |                           | inactive +------>
///        |        +<-- -- -- -- -- -- -- -- --+          |
///        +-+--+---+        reactivate         +-----+----+
///          |  ^     (*optional) automatically       ^
///          |  |                                     |
///          |  | unpark                              | automatically
/// slashing |  |                                     |
///          |  |             +--------+              |
///          |  +-------------+        |              |
///          |                | parked +--------------+
///          +--------------->+        |
///                           +--------+
///
/// (*optional) The validator my be set to automatically reactivate itself upon inactivation.
///             If this setting is not enabled the state change is triggered manually.
///
/// Create, Update, Retire, Re-activate and Unpark are incoming transactions to the staking contract.
/// Drop is an outgoing transaction from the staking contract.
/// To Create, Update or Drop, the cold key must be used (the one corresponding to the validator
/// address). For the other transactions, the the signing key must be used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validator {
    // The address of the validator. The corresponding key can be used to create, update or drop
    // the validator.
    pub address: Address,
    // This key used to sign blocks. It is also used to retire, reactivate and unpark the validator.
    pub signing_key: SchnorrPublicKey,
    // The voting key, it is used to vote for skip and macro blocks.
    pub voting_key: BlsPublicKey,
    // The reward address of the validator. All the block rewards are paid to this address.
    pub reward_address: Address,
    // Signalling field. Can be used to do chain upgrades or for any other purpose that requires
    // validators to coordinate among themselves.
    pub signal_data: Option<Blake2bHash>,
    // The amount of coins held by this validator. It also includes the coins delegated to him by
    // stakers.
    pub balance: Coin,
    // The amount of coins deposit by this validator. The initial deposit is a fixed amount,
    // however this value can be decremented by failing staking transactions due to fees.
    pub deposit: Coin,
    // The number of stakers that are staking for this validator.
    pub num_stakers: u64,
    // A flag stating if the validator is inactive. If it is inactive, then it contains the block
    // height at which it became inactive.
    pub inactivity_flag: Option<u32>,
}

impl StakingContract {
    /// Creates a new validator. The initial stake is always equal to the validator deposit
    /// and can only be retrieved by deleting the validator.
    /// This function is public to fill the genesis staking contract.
    /// The OperationInfo has always receipt = None, thus the type instationtion of the generic type to Receipt is irrelevant.
    pub fn create_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        signing_key: SchnorrPublicKey,
        voting_key: BlsPublicKey,
        reward_address: Address,
        signal_data: Option<Blake2bHash>,
        deposit: Coin,
    ) -> Result<OperationInfo<Receipt>, AccountError> {
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
            signing_key,
            voting_key,
            reward_address: reward_address.clone(),
            signal_data,
            balance: deposit,
            num_stakers: 0,
            inactivity_flag: None,
            deposit,
        };

        // All checks passed, not allowed to fail from here on!
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(OperationInfo {
            receipt: None,
            logs: vec![Log::CreateValidator {
                validator_address: validator_address.clone(),
                reward_address,
            }],
        })
    }

    /// Reverts creating a new validator entry.
    pub(crate) fn revert_create_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        reward_address: Address,
    ) -> Result<Vec<Log>, AccountError> {
        // Get the deposit value.
        let deposit = Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT);

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
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        accounts_tree.remove(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
        );

        Ok(vec![Log::CreateValidator {
            validator_address: validator_address.clone(),
            reward_address,
        }])
    }

    /// Updates some of the validator details (signing key, voting key, reward address and/or signal data).
    pub(crate) fn update_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        new_signing_key: Option<SchnorrPublicKey>,
        new_voting_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
    ) -> Result<OperationInfo<UpdateValidatorReceipt>, AccountError> {
        // Get the validator.
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                Some(v) => v,
                None => {
                    error!("Tried to update a validator that doesn't exist!");

                    return Ok(OperationInfo::with_receipt(
                        UpdateValidatorReceipt {
                            no_op: true,
                            old_signing_key: Default::default(),
                            old_voting_key: CompressedPublicKey::default(),
                            old_reward_address: Default::default(),
                            old_signal_data: None,
                        },
                        vec![],
                    ));
                }
            };

        // Create receipt now.
        let receipt = UpdateValidatorReceipt {
            no_op: false,
            old_signing_key: validator.signing_key,
            old_voting_key: validator.voting_key.clone(),
            old_reward_address: validator.reward_address.clone(),
            old_signal_data: validator.signal_data.clone(),
        };

        // Update validator info.
        if let Some(value) = new_signing_key {
            validator.signing_key = value;
        }

        if let Some(value) = new_voting_key {
            validator.voting_key = value;
        }

        if let Some(value) = new_reward_address {
            validator.reward_address = value;
        }

        if let Some(value) = new_signal_data {
            validator.signal_data = value;
        }

        // All checks passed, not allowed to fail from here on!
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator.clone()),
        );
        let logs = vec![Log::UpdateValidator {
            validator_address: validator_address.clone(),
            old_reward_address: receipt.old_reward_address.clone(),
            new_reward_address: Some(validator.reward_address),
        }];

        Ok(OperationInfo::with_receipt(receipt, logs))
    }

    /// Reverts updating validator details.
    pub(crate) fn revert_update_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        receipt: UpdateValidatorReceipt,
    ) -> Result<Vec<Log>, AccountError> {
        // If it was a no-op, we end right here.
        if receipt.no_op {
            return Ok(vec![]);
        }
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

        let log = Log::UpdateValidator {
            validator_address: validator_address.clone(),
            old_reward_address: receipt.old_reward_address.clone(),
            new_reward_address: Some(validator.reward_address),
        };

        // Revert validator info.
        validator.signing_key = receipt.old_signing_key;
        validator.voting_key = receipt.old_voting_key;
        validator.reward_address = receipt.old_reward_address;
        validator.signal_data = receipt.old_signal_data;

        // All checks passed, not allowed to fail from here on!
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(vec![log])
    }

    /// Inactivates a validator. It is necessary to retire a validator before dropping it. This also
    /// removes the validator from the parking set.
    pub(crate) fn inactivate_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        signer: &Address,
        block_height: u32,
    ) -> Result<OperationInfo<InactivateValidatorReceipt>, AccountError> {
        // Get the validator and check that the signature is valid.
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                Some(v) => v,
                None => {
                    error!("Tried to inactivate a validator that doesn't exist!");

                    return Ok(OperationInfo::with_receipt(
                        InactivateValidatorReceipt {
                            no_op: true,
                            parked_set: false,
                        },
                        vec![],
                    ));
                }
            };

        if *signer != Address::from(&validator.signing_key) {
            error!(
                "The key that signed the transaction doesn't match the signing key of the validator."
            );

            return Ok(OperationInfo::with_receipt(
                InactivateValidatorReceipt {
                    no_op: true,
                    parked_set: false,
                },
                vec![],
            ));
        }

        validator.inactivity_flag = Some(block_height);

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        if staking_contract
            .active_validators
            .remove(validator_address)
            .is_none()
        {
            info!(
                "Tried to inactivate a validator that was already inactivated! It has address {}.",
                validator_address
            );

            return Ok(OperationInfo::with_receipt(
                InactivateValidatorReceipt {
                    no_op: true,
                    parked_set: false,
                },
                vec![],
            ));
        }

        let parked_set = staking_contract.parked_set.remove(validator_address);

        // All checks passed, not allowed to fail from here on!
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(OperationInfo::with_receipt(
            InactivateValidatorReceipt {
                no_op: false,
                parked_set,
            },
            vec![Log::InactivateValidator {
                validator_address: validator_address.clone(),
            }],
        ))
    }

    /// Reverts inactivating a validator.
    pub(crate) fn revert_inactivate_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        receipt: InactivateValidatorReceipt,
    ) -> Result<Vec<Log>, AccountError> {
        // If it was a no-op, we end right here.
        if receipt.no_op {
            return Ok(vec![]);
        }

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
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(vec![Log::InactivateValidator {
            validator_address: validator_address.clone(),
        }])
    }

    /// Reactivates a validator.
    pub(crate) fn reactivate_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        signer: &Address,
    ) -> Result<OperationInfo<ReactivateValidatorReceipt>, AccountError> {
        // Get the validator and check that the signature is valid.
        let mut validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                Some(v) => v,
                None => {
                    error!("Tried to reactivate a validator that doesn't exist!");
                    return Ok(OperationInfo::with_receipt(
                        ReactivateValidatorReceipt {
                            no_op: true,
                            retire_time: 0,
                        },
                        vec![],
                    ));
                }
            };

        if *signer != Address::from(&validator.signing_key) {
            error!(
                "The key that signed the transaction doesn't match the signing key of the validator."
            );

            return Ok(OperationInfo::with_receipt(
                ReactivateValidatorReceipt {
                    no_op: true,
                    retire_time: 0,
                },
                vec![],
            ));
        }

        // Create receipt now.
        let receipt = match validator.inactivity_flag {
            Some(block_height) => ReactivateValidatorReceipt {
                no_op: false,
                retire_time: block_height,
            },
            None => {
                error!(
                    "Tried to reactivate a validator that was already active! It has address {}.",
                    validator_address
                );

                return Ok(OperationInfo::with_receipt(
                    ReactivateValidatorReceipt {
                        no_op: true,
                        retire_time: 0,
                    },
                    vec![],
                ));
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
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(OperationInfo::with_receipt(
            receipt,
            vec![Log::ReactivateValidator {
                validator_address: validator_address.clone(),
            }],
        ))
    }

    /// Reverts reactivating a validator.
    pub(crate) fn revert_reactivate_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        receipt: ReactivateValidatorReceipt,
    ) -> Result<Vec<Log>, AccountError> {
        // If it was a no-op, we end right here.
        if receipt.no_op {
            return Ok(vec![]);
        }

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
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator),
        );

        Ok(vec![Log::ReactivateValidator {
            validator_address: validator_address.clone(),
        }])
    }

    /// Removes a validator from the parked set and the disabled slots. This is used by validators
    /// after they get slashed so that they can produce blocks again.
    pub(crate) fn unpark_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        signer: &Address,
    ) -> Result<OperationInfo<UnparkValidatorReceipt>, AccountError> {
        // Get the validator and check that the signature is valid.
        let validator =
            match StakingContract::get_validator(accounts_tree, db_txn, validator_address) {
                Some(v) => v,
                None => {
                    error!("Tried to unpark a validator that doesn't exist!");
                    return Ok(OperationInfo::with_receipt(
                        UnparkValidatorReceipt {
                            no_op: true,
                            parked_set: false,
                            current_disabled_slots: None,
                            previous_disabled_slots: None,
                        },
                        vec![],
                    ));
                }
            };

        if *signer != Address::from(&validator.signing_key) {
            error!(
                "The key that signed the transaction doesn't match the signing key of the validator."
            );

            return Ok(OperationInfo::with_receipt(
                UnparkValidatorReceipt {
                    no_op: true,
                    parked_set: false,
                    current_disabled_slots: None,
                    previous_disabled_slots: None,
                },
                vec![],
            ));
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

        let mut logs = vec![];
        let no_op = if !parked_set && current_disabled.is_none() && previous_disabled.is_none() {
            error!(
                "Tried to unpark a validator that was already unparked! It has address {}.",
                validator_address
            );

            true
        } else {
            logs.push(Log::UnparkValidator {
                validator_address: validator_address.clone(),
            });
            false
        };

        // All checks passed, not allowed to fail from here on!
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(OperationInfo::with_receipt(
            UnparkValidatorReceipt {
                no_op,
                parked_set,
                current_disabled_slots: current_disabled,
                previous_disabled_slots: previous_disabled,
            },
            logs,
        ))
    }

    /// Reverts an unparking transaction.
    pub(crate) fn revert_unpark_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        receipt: UnparkValidatorReceipt,
    ) -> Result<Vec<Log>, AccountError> {
        // If it was a no-op, we end right here.
        if receipt.no_op {
            return Ok(vec![]);
        }

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
        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(vec![Log::UnparkValidator {
            validator_address: validator_address.clone(),
        }])
    }

    /// Delete a validator and returns its deposit. This can only be used on inactive validators!
    /// After the validator gets inactivated, it needs to wait until the second batch of the next
    /// epoch in order to be able to be deleted. This is necessary because if the validator was an
    /// elected validator when it was inactivated then it might receive rewards until the end of the
    /// first batch of the next epoch. So it needs to be available.
    pub(crate) fn delete_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        _: u32,
        transaction_total_value: Coin,
    ) -> Result<OperationInfo<DeleteValidatorReceipt>, AccountError> {
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
                    "Tried to delete a validator which was still active! Validator address {}",
                    validator_address
                );
                return Err(AccountError::InvalidForSender);
            }
            Some(_time) => {
                // This check was removed because of the nature of the failed delete validator transaction
                // (which can delete the validator if the deposit is consumed, regardless of the inactivity state)
                // However, if slashing is implemented, then this strategy needs to be revisited.
                // if block_height <= policy::election_block_after(time) + policy::BLOCKS_PER_BATCH {
                //    return Err(AccountError::InvalidForSender);
                //}
            }
        }

        let deposit = validator.deposit;

        // The transaction value + fee must be equal to the validator deposit
        if transaction_total_value != deposit {
            return Err(AccountError::InvalidCoinValue);
        }

        // All checks passed, not allowed to fail from here on!

        // Initialize the receipts and logs.
        let mut receipt = DeleteValidatorReceipt {
            signing_key: validator.signing_key,
            voting_key: validator.voting_key,
            reward_address: validator.reward_address.clone(),
            signal_data: validator.signal_data,
            retire_time: validator.inactivity_flag.expect(
                "This can't fail since we already checked above that the inactivity flag is Some.",
            ),
            stakers: vec![],
        };
        let logs = vec![Log::DeleteValidator {
            validator_address: validator_address.clone(),
            reward_address: validator.reward_address,
        }];
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

                    accounts_tree.put(
                        db_txn,
                        &StakingContract::get_key_staker(&staker_address),
                        Account::StakingStaker(staker),
                    );

                    // Remove the staker entry from the validator.
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

        staking_contract.balance = Account::balance_sub(staking_contract.balance, deposit)?;

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        // Return the receipt.
        Ok(OperationInfo::with_receipt(receipt, logs))
    }

    /// Reverts deleting a validator.
    pub(crate) fn revert_delete_validator(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        validator_address: &Address,
        receipt: DeleteValidatorReceipt,
    ) -> Result<Vec<Log>, AccountError> {
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
            staker.delegation = Some(validator_address.clone());

            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_staker(&staker_address),
                Account::StakingStaker(staker),
            );

            // Add the staker entry to the validator.
            accounts_tree.put(
                db_txn,
                &StakingContract::get_key_validator_staker(validator_address, &staker_address),
                Account::StakingValidatorsStaker(staker_address.clone()),
            );
        }

        // Re-add the validator entry.
        let validator = Validator {
            address: validator_address.clone(),
            signing_key: receipt.signing_key,
            voting_key: receipt.voting_key,
            reward_address: receipt.reward_address,
            signal_data: receipt.signal_data,
            balance: Coin::from_u64_unchecked(balance + Policy::VALIDATOR_DEPOSIT),
            num_stakers,
            inactivity_flag: Some(receipt.retire_time),
            deposit: Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT),
        };

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_validator(validator_address),
            Account::StakingValidator(validator.clone()),
        );

        // Get the staking contract main and update it.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let deposit = Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT);

        staking_contract.balance = Account::balance_add(staking_contract.balance, deposit)?;

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(vec![Log::DeleteValidator {
            validator_address: validator_address.clone(),
            reward_address: validator.reward_address,
        }])
    }
}
