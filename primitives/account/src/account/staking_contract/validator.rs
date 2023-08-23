use nimiq_bls::CompressedPublicKey as BlsPublicKey;
use nimiq_hash::Blake2bHash;
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_primitives::coin::Coin;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::{account::AccountError, policy::Policy};
use serde::{Deserialize, Serialize};

use crate::JailValidatorReceipt;
#[cfg(feature = "interaction-traits")]
use crate::{
    account::staking_contract::{
        receipts::{DeleteValidatorReceipt, ReactivateValidatorReceipt, UpdateValidatorReceipt},
        store::{
            StakingContractStoreReadOps, StakingContractStoreReadOpsExt, StakingContractStoreWrite,
        },
        StakingContract,
    },
    Log, RetireValidatorReceipt, TransactionLog,
};

/// Struct representing a validator in the staking contract.
/// Actions concerning a validator are:
/// 1. Create: Creates a validator.
/// 2. Update: Updates the validator.
/// 3. Deactivate: Deactivates a validator. This action is reversible. (starts a cooldown period used for Delete).
/// 4. Reactivate: Reactivates a validator.
/// 5. Delete: Deletes a validator (validator must have been inactive for the cooldown period).
/// 6. Retire: Permanently retires a validator. This action is required for deletion.
///
/// The actions can be summarized by the following state diagram:
///           +---+----+                            +---------+   
///   create  |        |            retire          |         |  delete
///+--------->+ active +--------------------------->+ retired +----------->
///           |        |                            |         |
///           +-+--+---+                            +----+----+
///             |  ^                                     ^
///             |  |                                     |
///    jail     |  | reactivate (*)                      |
///     or      |  | (**) optional automatic             |  retire
///  deactivate |  |              +----------+           |
///             |  +--------------+ inactive |           |
///             |                 |    or    +-----------+
///             +---------------->+  jailed  |
///                               +----------+
/// (*)  A validator can only reactivate itself if it has been released from jail or if it was not jailed
///      in the first place.
/// (**) The validator my be set to automatically reactivate itself upon inactivation.
///      If this setting is not enabled the state change can only be triggered manually.
///
/// Create, Update, Deactivate, Retire and Re-activate are incoming transactions to the staking contract.
/// Delete is an outgoing transaction from the staking contract.
/// To Create, Update or Delete, the cold key must be used (the one corresponding to the validator
/// address). For the other transactions, the the signing key must be used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validator {
    /// The address of the validator. The corresponding key can be used to create, update or drop
    /// the validator.
    pub address: Address,
    /// The public key used to sign blocks. It is also used to retire and reactivate the validator.
    pub signing_key: SchnorrPublicKey,
    /// The voting public key, it is used to vote for skip and macro blocks.
    pub voting_key: BlsPublicKey,
    /// The reward address of the validator. All the block rewards are paid to this address.
    pub reward_address: Address,
    /// Signaling field. Can be used to do chain upgrades or for any other purpose that requires
    /// validators to coordinate among themselves.
    pub signal_data: Option<Blake2bHash>,
    /// The total stake assigned to this validator. It includes the validator deposit as well as the
    /// coins delegated to him by stakers.
    pub total_stake: Coin,
    /// The amount of coins deposited by this validator. The initial deposit is a fixed amount,
    /// however this value can be decremented by failing staking transactions due to fees.
    pub deposit: Coin,
    /// The number of stakers that are delegating to this validator.
    pub num_stakers: u64,
    /// An option indicating if the validator is marked as inactive. If it is, then it contains the
    /// block height at which it becomes inactive.
    /// A validator can only effectively become inactive on the next election block. Thus, this may
    /// contain a block height in the future.
    pub inactive_from: Option<u32>,
    /// An option indicating if the validator is marked as jailed. If it is, then it contains the
    /// block height at which it becomes jailed.
    /// A validator can only effectively become jailed on the next election block. Thus, this may
    /// contain a block height in the future.
    pub jailed_from: Option<u32>,
    /// A flag indicating if the validator is retired.
    pub retired: bool,
}

impl Validator {
    pub fn is_active(&self) -> bool {
        self.inactive_from.is_none()
    }

    /// Checks if a validator is currently jailed.
    pub fn is_jailed(&self, block_number: u32) -> bool {
        if let Some(jailed_from) = self.jailed_from {
            return jailed_from <= block_number
                && block_number < Policy::block_after_jail(jailed_from);
        }
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub remaining_stake: Coin,
    pub num_remaining_stakers: u64,
}

#[cfg(feature = "interaction-traits")]
impl StakingContract {
    /// Creates a new validator. The initial stake is always equal to the validator deposit
    /// and can only be retrieved by deleting the validator.
    /// This function is public to fill the genesis staking contract.
    pub fn create_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        signing_key: SchnorrPublicKey,
        voting_key: BlsPublicKey,
        reward_address: Address,
        signal_data: Option<Blake2bHash>,
        deposit: Coin,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Fail if the validator already exists.
        if store.get_validator(validator_address).is_some() {
            return Err(AccountError::AlreadyExistentAddress {
                address: validator_address.clone(),
            });
        }
        // Fail if validator existed already.
        let tombstone = store.get_tombstone(validator_address);
        if tombstone.is_some() {
            return Err(AccountError::InvalidForRecipient);
        }

        // All checks passed, not allowed to fail from here on!

        // Initialize validator.
        let validator = Validator {
            address: validator_address.clone(),
            signing_key,
            voting_key,
            reward_address,
            signal_data,
            total_stake: deposit,
            deposit,
            num_stakers: 0,
            inactive_from: None,
            jailed_from: None,
            retired: false,
        };

        // Update our balance.
        self.balance += deposit;

        self.active_validators
            .insert(validator_address.clone(), validator.total_stake);

        tx_logger.push_log(Log::CreateValidator {
            validator_address: validator_address.clone(),
            reward_address: validator.reward_address.clone(),
        });

        // Create the validator entry.
        store.put_validator(validator_address, validator);

        Ok(())
    }

    /// Reverts creating a new validator entry.
    pub fn revert_create_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        deposit: Coin,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let validator = store.expect_validator(validator_address)?;

        // All checks passed, not allowed to fail from here on!

        // Update our balance.
        assert_eq!(validator.deposit, deposit);
        self.balance -= deposit;

        self.active_validators.remove(validator_address);

        // Remove the validator entry.
        store.remove_validator(validator_address);

        tx_logger.push_log(Log::CreateValidator {
            validator_address: validator_address.clone(),
            reward_address: validator.reward_address,
        });

        Ok(())
    }

    /// Updates some of the validator details (signing key, voting key, reward address and/or signal data).
    pub fn update_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        new_signing_key: Option<SchnorrPublicKey>,
        new_voting_key: Option<BlsPublicKey>,
        new_reward_address: Option<Address>,
        new_signal_data: Option<Option<Blake2bHash>>,
        tx_logger: &mut TransactionLog,
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

        tx_logger.push_log(Log::UpdateValidator {
            validator_address: validator_address.clone(),
            old_reward_address: receipt.old_reward_address.clone(),
            new_reward_address: Some(validator.reward_address.clone()),
        });

        // Update the validator entry.
        store.put_validator(validator_address, validator);

        Ok(receipt)
    }

    /// Reverts updating validator details.
    pub fn revert_update_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        receipt: UpdateValidatorReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        tx_logger.push_log(Log::UpdateValidator {
            validator_address: validator_address.clone(),
            old_reward_address: receipt.old_reward_address.clone(),
            new_reward_address: Some(validator.reward_address),
        });

        // Revert validator info.
        validator.signing_key = receipt.old_signing_key;
        validator.voting_key = receipt.old_voting_key;
        validator.reward_address = receipt.old_reward_address;
        validator.signal_data = receipt.old_signal_data;

        // Update the validator entry.
        store.put_validator(validator_address, validator);

        Ok(())
    }

    /// Deactivates a validator. It is necessary to retire a validator before dropping it.
    pub fn deactivate_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        signer: &Address,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

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
        // A validator can only effectively become inactive on the next election block.
        let next_election_block = Policy::election_block_after(block_number);
        validator.inactive_from = Some(next_election_block);

        // Remove validator from active_validators.
        // We expect the validator to be present since we checked that it is not inactive above.
        self.active_validators
            .remove(validator_address)
            .expect("inconsistent contract state");

        // Update validator entry.
        store.put_validator(validator_address, validator);

        tx_logger.push_log(Log::DeactivateValidator {
            validator_address: validator_address.clone(),
            inactive_from: next_election_block,
        });

        Ok(())
    }

    /// Reverts inactivating a validator.
    pub fn revert_deactivate_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // Mark validator as active.
        let inactive_from = validator
            .inactive_from
            .expect("must have inactivation date");
        validator.inactive_from = None;

        // Re-add validator to active_validators.
        self.active_validators
            .insert(validator_address.clone(), validator.total_stake);

        // Update validator entry.
        store.put_validator(validator_address, validator);

        tx_logger.push_log(Log::DeactivateValidator {
            validator_address: validator_address.clone(),
            inactive_from,
        });

        Ok(())
    }

    /// Deactivates a validator. It is necessary to retire a validator before dropping it.
    pub fn jail_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<JailValidatorReceipt, AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // All checks passed, not allowed to fail from here on!
        let old_jailed_from = validator.jailed_from;
        let newly_deactivated = validator.is_active();

        let next_election_block = Policy::election_block_after(block_number);

        // Mark validator as inactive.
        // A validator can only effectively become inactive on the next election block.
        if newly_deactivated {
            validator.inactive_from = Some(next_election_block);

            // Remove validator from active_validators.
            // We expect the validator to be present since we checked that it is not inactive above.
            self.active_validators
                .remove(validator_address)
                .expect("inconsistent contract state");
        }

        // Mark validator as Jailed.
        // A validator can only effectively become jailed on the next election block.
        validator.jailed_from = Some(next_election_block);

        // Update validator entry.
        store.put_validator(validator_address, validator);

        tx_logger.push_log(Log::JailValidator {
            validator_address: validator_address.clone(),
            jailed_from: next_election_block,
        });
        if newly_deactivated {
            tx_logger.push_log(Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from: next_election_block,
            });
        }

        Ok(JailValidatorReceipt {
            newly_deactivated,
            old_jailed_from,
        })
    }

    /// Reverts inactivating a validator.
    pub fn revert_jail_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        receipt: JailValidatorReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        let inactive_from = validator.inactive_from;

        // Mark validator as active.
        if receipt.newly_deactivated {
            validator.inactive_from = None;

            // Re-add validator to active_validators.
            self.active_validators
                .insert(validator_address.clone(), validator.total_stake);
        }

        // Reset jail release.
        let jailed_from = validator.jailed_from.expect("must have jail date");
        validator.jailed_from = receipt.old_jailed_from;

        // Update validator entry.
        store.put_validator(validator_address, validator);

        tx_logger.push_log(Log::DeactivateValidator {
            validator_address: validator_address.clone(),
            inactive_from: inactive_from.expect("must have inactivation date"),
        });
        tx_logger.push_log(Log::JailValidator {
            validator_address: validator_address.clone(),
            jailed_from,
        });

        Ok(())
    }

    /// Reactivates a validator.
    pub fn reactivate_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        signer: &Address,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<ReactivateValidatorReceipt, AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // Check that the validator is inactive.
        if validator.is_active() {
            debug!("Validator {} is already active", validator_address);
            return Err(AccountError::InvalidForRecipient);
        }

        // Check that the validator is not retired.
        if validator.retired {
            debug!("Validator {} is retired", validator_address);
            return Err(AccountError::InvalidForRecipient);
        }

        // Check that the validator is not jailed.
        if validator.is_jailed(block_number) {
            debug!(
                "Validator {} is jailed since {}",
                validator_address,
                validator.jailed_from.unwrap()
            );
            return Err(AccountError::InvalidForRecipient);
        }

        // Check that the signer is correct.
        if *signer != Address::from(&validator.signing_key) {
            debug!("The transaction signer doesn't match the signing key of the validator.");
            return Err(AccountError::InvalidSignature);
        }

        // All checks passed, not allowed to fail from here on!

        // Mark validator as active.
        let was_inactive_from = validator
            .inactive_from
            .take()
            .expect("validator is inactive");

        // Add validator to active_validators.
        self.active_validators
            .insert(validator_address.clone(), validator.total_stake);

        // Update validator entry.
        store.put_validator(validator_address, validator);

        tx_logger.push_log(Log::ReactivateValidator {
            validator_address: validator_address.clone(),
        });

        Ok(ReactivateValidatorReceipt { was_inactive_from })
    }

    /// Reverts reactivating a validator.
    pub fn revert_reactivate_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        receipt: ReactivateValidatorReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // Restore validator inactive state.
        validator.inactive_from = Some(receipt.was_inactive_from);

        // Remove validator from active_validators again.
        self.active_validators
            .remove(validator_address)
            .expect("inconsistent contract state");

        // Update validator entry.
        store.put_validator(validator_address, validator);

        tx_logger.push_log(Log::ReactivateValidator {
            validator_address: validator_address.clone(),
        });

        Ok(())
    }

    /// Retires a validator, permanently deactivating it.
    /// It is necessary to retire a validator before deleting it.
    pub fn retire_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        block_number: u32,
        tx_logger: &mut TransactionLog,
    ) -> Result<RetireValidatorReceipt, AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // Check that the validator is not already retired.
        if validator.retired {
            debug!("Validator {} is already retired", validator_address);
            return Err(AccountError::InvalidForRecipient);
        }

        // All checks passed, not allowed to fail from here on!

        // Retire the validator.
        validator.retired = true;
        tx_logger.push_log(Log::RetireValidator {
            validator_address: validator_address.clone(),
        });

        // Mark validator as inactive if it is still active.
        let was_active = validator.is_active();
        if was_active {
            // A validator can only effectively become inactive on the next election block.
            let next_election_block = Policy::election_block_after(block_number);
            validator.inactive_from = Some(next_election_block);

            // Remove validator from active_validators.
            // We expect the validator to be present since we checked that it is not inactive above.
            self.active_validators
                .remove(validator_address)
                .expect("inconsistent contract state");

            tx_logger.push_log(Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from: next_election_block,
            });
        }

        // Update validator entry.
        store.put_validator(validator_address, validator);

        Ok(RetireValidatorReceipt { was_active })
    }

    /// Reverts retiring a validator.
    pub fn revert_retire_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        receipt: RetireValidatorReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Get the validator.
        let mut validator = store.expect_validator(validator_address)?;

        // Clear the retired flag.
        validator.retired = false;

        // Reactivate validator if it was active before.
        if receipt.was_active {
            // Mark validator as active.
            let inactive_from = validator
                .inactive_from
                .expect("must have inactivation date");
            validator.inactive_from = None;

            // Re-add validator to active_validators.
            self.active_validators
                .insert(validator_address.clone(), validator.total_stake);

            tx_logger.push_log(Log::DeactivateValidator {
                validator_address: validator_address.clone(),
                inactive_from,
            });
        }

        tx_logger.push_log(Log::RetireValidator {
            validator_address: validator_address.clone(),
        });

        // Update validator entry.
        store.put_validator(validator_address, validator);

        Ok(())
    }

    /// Checks if a validator can be deleted.
    pub fn can_delete_validator(
        &self,
        validator: &Validator,
        block_number: u32,
    ) -> Result<(), AccountError> {
        // Check that the validator is retired.
        if !validator.retired {
            debug!("Tried to delete active validator {}", validator.address);
            return Err(AccountError::InvalidForSender);
        }

        // Check that the validator is not currently jailed.
        if validator.is_jailed(block_number) {
            debug!(
                "Validator {} is jailed since {}",
                validator.address,
                validator.jailed_from.unwrap()
            );
            return Err(AccountError::InvalidForSender);
        }

        // Check that the validator has been inactive for long enough.
        // We must wait until all potential offenses of this validator could have been reported.
        // This includes the time it takes for the deactivation to take effect (end of the epoch)
        // and the reporting window from there.
        let inactive_from = validator
            .inactive_from
            .expect("Validator is retired so it must be inactive");
        let wait_until = Policy::last_block_of_reporting_window(inactive_from);
        if block_number <= wait_until {
            debug!(
                current_block = block_number,
                wait_until, "Tried to delete validator {} too soon", validator.address
            );
            return Err(AccountError::InvalidForSender);
        }

        Ok(())
    }

    /// Deletes a validator and returns its deposit. This can only be used on retired validators!
    /// After the validator gets deactivated, it needs to wait until the second batch of the next
    /// epoch in order to be able to be deleted. This is necessary because if the validator was an
    /// elected validator when it was deactivated then it might receive rewards until the end of the
    /// first batch of the next epoch. So it needs to be available.
    ///
    /// When a validator is deleted, the stakers delegating to it will NOT be updated. If there is
    /// at least one staker for a validator, we leave a tombstone for it behind that tracks the
    /// remaining total_stake. This is necessary to be able to correctly restore the validator entry
    /// in case it is created again. The tombstone is deleted once the last delegation to the
    /// deleted validator is removed (e.g. by update_staker or remove_stake).
    pub fn delete_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        block_number: u32,
        transaction_total_value: Coin,
        tx_logger: &mut TransactionLog,
    ) -> Result<DeleteValidatorReceipt, AccountError> {
        // Get the validator.
        let validator = store.expect_validator(validator_address)?;

        // Check that the validator can be deleted.
        self.can_delete_validator(&validator, block_number)?;

        // The transaction value + fee must be equal to the validator deposit
        if transaction_total_value != validator.deposit {
            return Err(AccountError::InvalidCoinValue);
        }

        // All checks passed, not allowed to fail from here on!

        // Update our balance.
        self.balance -= validator.deposit;

        // If there are stakers remaining, create a tombstone for this validator.
        if validator.num_stakers > 0 {
            let tombstone = Tombstone {
                remaining_stake: validator.total_stake - validator.deposit,
                num_remaining_stakers: validator.num_stakers,
            };
            store.put_tombstone(validator_address, tombstone);
        }

        // Remove the validator entry.
        store.remove_validator(validator_address);

        tx_logger.push_log(Log::DeleteValidator {
            validator_address: validator_address.clone(),
            reward_address: validator.reward_address.clone(),
        });

        // Return the receipt.
        Ok(DeleteValidatorReceipt {
            signing_key: validator.signing_key,
            voting_key: validator.voting_key,
            reward_address: validator.reward_address,
            signal_data: validator.signal_data,
            inactive_from: validator.inactive_from.unwrap(), // we checked above that this is Some
            jailed_from: validator.jailed_from,
        })
    }

    /// Reverts deleting a validator.
    pub fn revert_delete_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        transaction_total_value: Coin,
        receipt: DeleteValidatorReceipt,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        // Update our balance.
        self.balance += transaction_total_value;

        // Initialize validator.
        let mut validator = Validator {
            address: validator_address.clone(),
            signing_key: receipt.signing_key,
            voting_key: receipt.voting_key,
            reward_address: receipt.reward_address,
            signal_data: receipt.signal_data,
            total_stake: transaction_total_value,
            deposit: transaction_total_value,
            num_stakers: 0,
            inactive_from: Some(receipt.inactive_from),
            jailed_from: receipt.jailed_from,
            retired: true,
        };

        // If there is a tombstone for this validator, add the remaining staker and stakers.
        if let Some(tombstone) = store.get_tombstone(validator_address) {
            validator.total_stake += tombstone.remaining_stake;
            validator.num_stakers += tombstone.num_remaining_stakers;

            // Remove the tombstone entry.
            store.remove_tombstone(validator_address);
        }

        tx_logger.push_log(Log::DeleteValidator {
            validator_address: validator_address.clone(),
            reward_address: validator.reward_address.clone(),
        });

        // Re-add the validator entry.
        store.put_validator(validator_address, validator);

        Ok(())
    }
}
