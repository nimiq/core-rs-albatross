use std::cmp::min;

use log::error;

use beserial::{Deserialize, Serialize};
use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_primitives::{account::AccountError, coin::Coin, policy::Policy};

use crate::account::staking_contract::receipts::{
    DeleteValidatorReceipt, InactivateValidatorReceipt, ReactivateValidatorReceipt,
    UnparkValidatorReceipt, UpdateValidatorReceipt,
};
use crate::account::staking_contract::store::{
    StakingContractStoreReadOps, StakingContractStoreReadOpsExt, StakingContractStoreWrite,
};
use crate::logs::{Log, OperationInfo};
use crate::{account::staking_contract::StakingContract, Account};

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
    // Signaling field. Can be used to do chain upgrades or for any other purpose that requires
    // validators to coordinate among themselves.
    pub signal_data: Option<Blake2bHash>,
    // The total stake assigned to this validator. It includes the validator deposit as well as the
    // coins delegated to him by stakers.
    pub total_stake: Coin,
    // The amount of coins deposited by this validator. The initial deposit is a fixed amount,
    // however this value can be decremented by failing staking transactions due to fees.
    pub deposit: Coin,
    // The number of stakers that are delegating to this validator.
    pub num_stakers: u64,
    // An option indicating if the validator is inactive. If it is inactive, then it contains the
    // block height at which it became inactive.
    pub inactive_since: Option<u32>,
}

impl Validator {
    fn is_active(&self) -> bool {
        self.inactive_since.is_none()
    }
}

impl StakingContract {
    /// Creates a new validator. The initial stake is always equal to the validator deposit
    /// and can only be retrieved by deleting the validator.
    /// This function is public to fill the genesis staking contract.
    pub fn create_validator(
        &mut self,
        store: &StakingContractStoreWrite,
        validator_address: &Address,
        signing_key: SchnorrPublicKey,
        voting_key: BlsPublicKey,
        reward_address: Address,
        signal_data: Option<Blake2bHash>,
        deposit: Coin,
    ) -> Result<(), AccountError> {
        // Fail if the validator already exists.
        if store.get_validator(validator_address).is_some() {
            return Err(AccountError::AlreadyExistentAddress {
                address: validator_address.clone(),
            });
        }

        // All checks passed, not allowed to fail from here on!

        // Update our balance.
        Account::balance_add_assign(&mut self.balance, deposit)?;

        self.active_validators
            .insert(validator_address.clone(), deposit);

        // Create validator struct.
        let validator = Validator {
            address: validator_address.clone(),
            signing_key,
            voting_key,
            reward_address,
            signal_data,
            total_stake: deposit,
            num_stakers: 0,
            inactive_since: None,
            deposit,
        };

        // Create the validator entry.
        store.put_validator(validator_address, validator);

        Ok(())
    }

    /// Reverts creating a new validator entry.
    pub fn revert_create_validator(
        &mut self,
        store: &StakingContractStoreWrite,
        validator_address: &Address,
        deposit: Coin,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let validator = store.expect_validator(validator_address)?;

        // All checks passed, not allowed to fail from here on!

        // Update our balance.
        assert_eq!(validator.deposit, deposit);
        Account::balance_sub_assign(&mut self.balance, deposit)?;

        self.active_validators.remove(validator_address);

        // Remove the validator entry.
        store.remove_validator(validator_address);

        Ok(())
    }

    /// Updates some of the validator details (signing key, voting key, reward address and/or signal data).
    pub fn update_validator(
        &mut self,
        store: &StakingContractStoreWrite,
        validator_address: &Address,
        new_signing_key: Option<SchnorrPublicKey>,
        new_voting_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
    ) -> Result<UpdateValidatorReceipt, AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // Create the receipt.
        let receipt = UpdateValidatorReceipt {
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

        // Update the validator entry.
        store.put_validator(validator_address, validator);

        // let log = Log::UpdateValidator {
        //     validator_address: validator_address.clone(),
        //     old_reward_address: receipt.old_reward_address.clone(),
        //     new_reward_address: Some(validator.reward_address),
        // };

        Ok(receipt)
    }

    /// Reverts updating validator details.
    pub fn revert_update_validator(
        &mut self,
        store: &StakingContractStoreWrite,
        validator_address: &Address,
        receipt: UpdateValidatorReceipt,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // Revert validator info.
        validator.signing_key = receipt.old_signing_key;
        validator.voting_key = receipt.old_voting_key;
        validator.reward_address = receipt.old_reward_address;
        validator.signal_data = receipt.old_signal_data;

        // Update the validator entry.
        store.put_validator(validator_address, validator);

        Ok(())
    }

    /// Inactivates a validator. It is necessary to retire a validator before dropping it. This also
    /// removes the validator from the parking set.
    pub fn inactivate_validator(
        &mut self,
        store: &StakingContractStoreWrite,
        validator_address: &Address,
        signer: &Address,
        block_number: u32,
    ) -> Result<InactivateValidatorReceipt, AccountError> {
        // Get the validator.
        let mut validator = store.get_validator(validator_address)?;

        // Check that the validator is active.
        if !validator.is_active() {
            debug!("Validator {} is already inactive", validator_address);
            return Err(AccountError::InvalidForRecipient);
        }

        // Check that the signer is correct.
        if *signer != Address::from(&validator.signing_key) {
            debug!("The transaction signer doesn't match the signing key of the validator.");
            return Err(AccountError::InvalidSignature);
        }

        // All checks passed, not allowed to fail from here on!

        // Mark validator as inactive.
        validator.inactive_since = Some(block_number);

        // Remove validator from active_validators.
        // We expect the validator to be present since we checked that it is not inactive above.
        self.active_validators
            .remove(validator_address)
            .expect("inconsistent contract state");

        // Remove validator from parked_set.
        let was_parked = self.parked_set.remove(validator_address);

        // Update validator entry.
        store.put_validator(validator_address, validator);

        Ok(InactivateValidatorReceipt { was_parked })
    }

    /// Reverts inactivating a validator.
    pub fn revert_inactivate_validator(
        &mut self,
        store: &StakingContractStoreWrite,
        validator_address: &Address,
        receipt: InactivateValidatorReceipt,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // Mark validator as active.
        validator.inactive_since = None;

        // Re-add validator to active_validators.
        self.active_validators
            .insert(validator_address.clone(), validator.total_stake);

        // Re-add validator to parked_set if it was parked before.
        if receipt.was_parked {
            self.parked_set.insert(validator_address.clone());
        }

        // Update validator entry.
        store.put_validator(validator_address, validator);

        Ok(())
    }

    /// Reactivates a validator.
    pub fn reactivate_validator(
        &mut self,
        store: &StakingContractStoreWrite,
        validator_address: &Address,
        signer: &Address,
    ) -> Result<ReactivateValidatorReceipt, AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // Check that the validator is inactive.
        if validator.is_active() {
            debug!("Validator {} is already active", validator_address);
            return Err(AccountError::InvalidForRecipient);
        }

        // Check that the signer is correct.
        if *signer != Address::from(&validator.signing_key) {
            debug!("The transaction signer doesn't match the signing key of the validator.");
            return Err(AccountError::InvalidSignature);
        }

        // All checks passed, not allowed to fail from here on!

        // Mark validator as active.
        let was_inactive_since = validator
            .inactive_since
            .take()
            .expect("validator is inactive");

        // Add validator to active_validators.
        self.active_validators
            .insert(validator_address.clone(), validator.total_stake);

        // Update validator entry.
        store.put_validator(validator_address, validator);

        Ok(ReactivateValidatorReceipt { was_inactive_since })
    }

    /// Reverts reactivating a validator.
    pub fn revert_reactivate_validator(
        &mut self,
        store: &StakingContractStoreWrite,
        validator_address: &Address,
        receipt: ReactivateValidatorReceipt,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // Restore validator inactive state.
        validator.inactive_since = Some(receipt.was_inactive_since);

        // Remove validator from active_validators again.
        self.active_validators
            .remove(validator_address)
            .expect("inconsistent contract state");

        // Update validator entry.
        store.put_validator(validator_address, validator);

        Ok(())
    }

    /// Reverts an unpark transaction.
    pub fn revert_unpark_validator(
        &mut self,
        store: &StakingContractStoreWrite,
        validator_address: &Address,
        receipt: UnparkValidatorReceipt,
    ) -> Result<(), AccountError> {
        // Re-add validator to parked_set.
        self.parked_set.insert(validator_address.clone());

        // Re-add current and previous disabled slots.
        if let Some(slots) = receipt.current_disabled_slots {
            self.current_disabled_slots
                .insert(validator_address.clone(), slots);
        }
        if let Some(slots) = receipt.previous_disabled_slots {
            self.previous_disabled_slots
                .insert(validator_address.clone(), slots);
        }

        Ok(())
    }

    /// Delete a validator and returns its deposit. This can only be used on inactive validators!
    /// After the validator gets inactivated, it needs to wait until the second batch of the next
    /// epoch in order to be able to be deleted. This is necessary because if the validator was an
    /// elected validator when it was inactivated then it might receive rewards until the end of the
    /// first batch of the next epoch. So it needs to be available.
    pub fn delete_validator(
        &mut self,
        store: &StakingContractStoreWrite,
        validator_address: &Address,
        block_number: u32,
        transaction_total_value: Coin,
    ) -> Result<OperationInfo<DeleteValidatorReceipt>, AccountError> {
        // Get the validator.
        let validator = store.expect_validator(validator_address)?;

        // Check that the validator has been inactive for long enough.
        if let Some(inactive_since) = validator.inactive_since {
            let deadline =
                Policy::election_block_after(inactive_since) + Policy::blocks_per_batch();
            if block_number <= deadline {
                debug!("Tried to delete validator {} too soon", validator_address);
                return Err(AccountError::InvalidForSender);
            }
        } else {
            debug!("Tried to delete active validator {}", validator_address);
            return Err(AccountError::InvalidForSender);
        }

        // The transaction value + fee must be equal to the validator deposit
        if transaction_total_value != validator.deposit {
            return Err(AccountError::InvalidCoinValue);
        }

        // All checks passed, not allowed to fail from here on!

        // Initialize receipt.
        let mut receipt = DeleteValidatorReceipt {
            signing_key: validator.signing_key,
            voting_key: validator.voting_key,
            reward_address: validator.reward_address,
            signal_data: validator.signal_data,
            inactive_since: validator.inactive_since.expect("validator is inactive"),
            stakers: vec![],
        };

        // let logs = vec![Log::DeleteValidator {
        //     validator_address: validator_address.clone(),
        //     reward_address: validator.reward_address,
        // }];

        // Remove the validator from all its stakers.
        // Also delete all the validator's delegation entries.
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
                    let mut staker = StakingContract::get_staker(accounts_tree, db_txn, &staker_address).expect("A validator had an staker delegating to it that doesn't exist in the Accounts Tree!");

                    staker.delegation = None;

                    accounts_tree
                        .put(
                            db_txn,
                            &StakingContract::get_key_staker(&staker_address),
                            Account::StakingStaker(staker),
                        )
                        .expect("temporary until accounts rewrite");

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
        let mut staking_contract = complete!(StakingContract::get_staking_contract_or_update(
            accounts_tree,
            db_txn
        ));

        staking_contract.balance = Account::balance_sub(staking_contract.balance, deposit)?;

        accounts_tree
            .put(
                db_txn,
                &StakingContract::get_key_staking_contract(),
                Account::Staking(staking_contract),
            )
            .expect("temporary until accounts rewrite");

        // Return the receipt.
        Ok(OperationInfo::with_receipt(receipt, logs))
    }

    /// Reverts deleting a validator.
    pub fn revert_delete_validator(
        &mut self,
        store: &StakingContractStoreWrite,
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
                "A validator had an staker delegating to it that doesn't exist in the Accounts Tree!",
            );

            // Update the counters.
            num_stakers += 1;
            balance += u64::from(staker.balance);

            // Update the staker.
            staker.delegation = Some(validator_address.clone());

            accounts_tree
                .put(
                    db_txn,
                    &StakingContract::get_key_staker(&staker_address),
                    Account::StakingStaker(staker),
                )
                .expect("temporary until accounts rewrite");

            // Add the staker entry to the validator.
            accounts_tree
                .put(
                    db_txn,
                    &StakingContract::get_key_validator_staker(validator_address, &staker_address),
                    Account::StakingValidatorsStaker(staker_address.clone()),
                )
                .expect("temporary until accounts rewrite");
        }

        // Re-add the validator entry.
        let validator = Validator {
            address: validator_address.clone(),
            signing_key: receipt.signing_key,
            voting_key: receipt.voting_key,
            reward_address: receipt.reward_address,
            signal_data: receipt.signal_data,
            total_stake: Coin::from_u64_unchecked(balance + policy::VALIDATOR_DEPOSIT),
            num_stakers,
            inactive_since: Some(receipt.inactive_since),
            deposit: Coin::from_u64_unchecked(policy::VALIDATOR_DEPOSIT),
        };

        accounts_tree
            .put(
                db_txn,
                &StakingContract::get_key_validator(validator_address),
                Account::StakingValidator(validator.clone()),
            )
            .expect("temporary until accounts rewrite");

        // Get the staking contract main and update it.
        let mut staking_contract = complete!(StakingContract::get_staking_contract_or_update(
            accounts_tree,
            db_txn
        ));

        let deposit = Coin::from_u64_unchecked(Policy::VALIDATOR_DEPOSIT);

        staking_contract.balance = Account::balance_add(staking_contract.balance, deposit)?;

        accounts_tree
            .put(
                db_txn,
                &StakingContract::get_key_staking_contract(),
                Account::Staking(staking_contract),
            )
            .expect("temporary until accounts rewrite");

        Ok(vec![Log::DeleteValidator {
            validator_address: validator_address.clone(),
            reward_address: validator.reward_address,
        }])
    }
}

/// Removes a validator from the parked set and the disabled slots. This is used by validators
/// after they get slashed so that they can produce blocks again.
pub fn unpark_validator(
    &mut self,
    store: &StakingContractStoreWrite,
    validator_address: &Address,
    signer: &Address,
) -> Result<UnparkValidatorReceipt, AccountError> {
    // Get the validator.
    let validator = store.expect_validator(validator_address)?;

    // Check that the validator is currently parked.
    if !self.parked_set.contains(validator_address) {
        debug!("Validator {} is not parked", validator_address);
        return Err(AccountError::InvalidForRecipient);
    }

    // Check that the signer is correct.
    if *signer != Address::from(&validator.signing_key) {
        debug!("The transaction signer doesn't match the signing key of the validator.");
        return Err(AccountError::InvalidSignature);
    }

    // Remove the validator from the parked_set.
    self.parked_set.remove(validator_address);

    // Clear the validators current and previous disabled slots.
    let current_disabled_slots = self.current_disabled_slots.remove(validator_address);
    let previous_disabled_slots = self.previous_disabled_slots.remove(validator_address);

    // logs.push(Log::UnparkValidator {
    //     validator_address: validator_address.clone(),
    // });

    Ok(UnparkValidatorReceipt {
        current_disabled_slots,
        previous_disabled_slots,
    })
}
