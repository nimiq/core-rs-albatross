use nimiq_keys::Address;
#[cfg(feature = "interaction-traits")]
use nimiq_primitives::account::AccountError;
use nimiq_primitives::coin::Coin;
use serde::{Deserialize, Serialize};

#[cfg(feature = "interaction-traits")]
use crate::{
    account::staking_contract::{
        store::{
            StakingContractStoreReadOps, StakingContractStoreReadOpsExt, StakingContractStoreWrite,
        },
        StakerReceipt, StakingContract, Tombstone,
    },
    Log, TransactionLog,
};

/// Struct representing a staker in the staking contract.
/// Actions concerning a staker are:
/// 1. Create: Creates a staker.
/// 2. Stake: Adds coins from any outside address to a staker's balance.
/// 3. Update: Updates the validator.
/// 4. Unstake: Removes coins from a staker's balance to outside the staking contract.
///
/// Create, Stake and Update are incoming transactions to the staking contract.
/// Unstake is an outgoing transaction from the staking contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staker {
    /// The address of the staker. The corresponding key is used for all transactions (except Stake
    /// which is open to any address).
    pub address: Address,
    /// The staker's balance.
    pub balance: Coin,
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

        // Create the staker struct.
        let staker = Staker {
            address: staker_address.clone(),
            balance: value,
            delegation,
        };

        // If we are delegating to a validator, we need to update it.
        if staker.delegation.is_some() {
            self.add_staker_to_validator(store, &staker)?;
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
        if staker.delegation.is_some() {
            self.remove_staker_from_validator(store, &staker)
                .expect("inconsistent contract state");
        }

        // Remove the staker entry.
        store.remove_staker(staker_address);

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

            // Update the validator.
            self.add_stake_to_validator(store, validator_address, value)?;
        }

        // Update the staker's balance.
        staker.balance += value;

        // Update our balance.
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
            self.remove_stake_from_validator(store, validator_address, value)
                .expect("inconsistent contract state");
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

    /// Updates the staker details. Right now you can only update the delegation.
    pub fn update_staker(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        delegation: Option<Address>,
        tx_logger: &mut TransactionLog,
    ) -> Result<StakerReceipt, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Check that the delegated validator exists.
        if let Some(new_validator_address) = &delegation {
            store.expect_validator(new_validator_address)?;
        }

        // All checks passed, not allowed to fail from here on!

        // Create logs.
        tx_logger.push_log(Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: staker.delegation.clone(),
            new_validator_address: delegation.clone(),
        });

        // Create the receipt.
        let receipt = StakerReceipt {
            delegation: staker.delegation.clone(),
        };

        // If we were delegating to a validator, we remove ourselves from it.
        if staker.delegation.is_some() {
            self.remove_staker_from_validator(store, &staker)
                .expect("inconsistent contract state");
        }

        // Update the staker's delegation.
        staker.delegation = delegation;

        // If we are now delegating to a validator, we add ourselves to it.
        if staker.delegation.is_some() {
            self.add_staker_to_validator(store, &staker)
                .expect("inconsistent contract state");
        }

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
        if staker.delegation.is_some() {
            self.remove_staker_from_validator(store, &staker)
                .expect("inconsistent contract state");
        }

        // Create logs.
        tx_logger.push_log(Log::UpdateStaker {
            staker_address: staker_address.clone(),
            old_validator_address: receipt.delegation.clone(),
            new_validator_address: staker.delegation.clone(),
        });

        // Restore the previous delegation.
        staker.delegation = receipt.delegation;

        // Add ourselves to the previous delegation, if it existed.
        if staker.delegation.is_some() {
            self.add_staker_to_validator(store, &staker)
                .expect("inconsistent contract state");
        }

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Removes coins from a staker's balance. If the entire staker's balance is removed then the
    /// staker is deleted.
    pub fn remove_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<StakerReceipt>, AccountError> {
        // Get the staker.
        let mut staker = store.expect_staker(staker_address)?;

        // Compute the new balance of the staker. We can't update `staker` here yet as
        // `remove_staker_from_validator` needs the original balance intact.
        let new_balance = staker.balance.safe_sub(value)?;

        // All checks passed, not allowed to fail from here on!

        // If we are delegating to a validator, we update it.
        if let Some(validator_address) = &staker.delegation {
            if new_balance.is_zero() {
                self.remove_staker_from_validator(store, &staker)
                    .expect("inconsistent contract state");
            } else {
                self.remove_stake_from_validator(store, validator_address, value)
                    .expect("inconsistent contract state");
            }
        }

        // Update the staker's balance.
        staker.balance = new_balance;

        // Update our balance.
        self.balance -= value;

        tx_logger.push_log(Log::Unstake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
        });

        // Update or remove the staker entry, depending on remaining balance.
        if staker.balance.is_zero() {
            store.remove_staker(staker_address);

            Ok(Some(StakerReceipt {
                delegation: staker.delegation,
            }))
        } else {
            store.put_staker(staker_address, staker);

            Ok(None)
        }
    }

    /// Reverts a remove_stake transaction.
    pub fn revert_remove_stake(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker_address: &Address,
        value: Coin,
        receipt: Option<StakerReceipt>,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        let mut staker = store
            .get_staker(staker_address)
            .or_else(|| {
                receipt.map(|receipt| {
                    // Set the staker balance to zero here, it is updated later.
                    let staker = Staker {
                        address: staker_address.clone(),
                        balance: Coin::ZERO,
                        delegation: receipt.delegation,
                    };

                    // If we are delegating to a validator, re-add the staker to it.
                    if staker.delegation.is_some() {
                        self.add_staker_to_validator(store, &staker)
                            .expect("inconsistent contract state");
                    }

                    staker
                })
            })
            .ok_or(AccountError::InvalidReceipt)?;

        // Update the staker's balance.
        staker.balance += value;

        // Update our balance.
        self.balance += value;

        // If we are delegating to a validator, we update it.
        if let Some(validator_address) = &staker.delegation {
            self.add_stake_to_validator(store, validator_address, value)
                .expect("inconsistent contract state");
        }

        tx_logger.push_log(Log::Unstake {
            staker_address: staker_address.clone(),
            validator_address: staker.delegation.clone(),
            value,
        });

        // Update the staker entry.
        store.put_staker(staker_address, staker);

        Ok(())
    }

    /// Adds a new staker to the validator given in staker.delegation.
    /// Panics if staker.delegation is None.
    fn add_staker_to_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker: &Staker,
    ) -> Result<(), AccountError> {
        let validator_address = staker
            .delegation
            .as_ref()
            .expect("Staker has no delegation");

        // Try to get the validator. It might have been deleted.
        if let Some(mut validator) = store.get_validator(validator_address) {
            // Validator exists, update it.
            validator.total_stake += staker.balance;

            if validator.is_active() {
                self.active_validators
                    .insert(validator_address.clone(), validator.total_stake);
            }

            validator.num_stakers += 1;

            // Update the validator entry.
            store.put_validator(validator_address, validator);

            return Ok(());
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.remaining_stake += staker.balance;

            tombstone.num_remaining_stakers += 1;

            store.put_tombstone(validator_address, tombstone);

            return Ok(());
        }

        // Tombstone doesn't exist, so it must have been deleted by a previous
        // `remove_staker_from_validator` call. Recreate it.
        // TODO We should consider guarding this functionality behind a flag. It's not obvious from
        //  the function name that this will create a tombstone if the validator doesn't exist.
        let tombstone = Tombstone {
            remaining_stake: staker.balance,
            num_remaining_stakers: 1,
        };
        store.put_tombstone(validator_address, tombstone);

        Ok(())
    }

    /// Removes a staker from the validator given in staker.delegation.
    /// Panics if staker.delegation is None.
    fn remove_staker_from_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        staker: &Staker,
    ) -> Result<(), AccountError> {
        let validator_address = staker
            .delegation
            .as_ref()
            .expect("Staker has no delegation");

        // Try to get the validator. It might have been deleted.
        if let Some(mut validator) = store.get_validator(validator_address) {
            // Validator exists, update it.
            validator.total_stake -= staker.balance;

            if validator.is_active() {
                self.active_validators
                    .insert(validator_address.clone(), validator.total_stake);
            }

            validator.num_stakers -= 1;

            // Update the validator entry.
            store.put_validator(validator_address, validator);

            return Ok(());
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.remaining_stake -= staker.balance;

            tombstone.num_remaining_stakers -= 1;

            // Delete the tombstone if this was the last remaining staker, update it otherwise.
            if tombstone.num_remaining_stakers == 0 {
                store.remove_tombstone(validator_address);
            } else {
                store.put_tombstone(validator_address, tombstone);
            }

            return Ok(());
        }

        // Neither validator nor tombstone exist, this is an error.
        panic!("inconsistent contract state");
    }

    /// Adds `value` coins to a given validator's total stake.
    fn add_stake_to_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        value: Coin,
    ) -> Result<(), AccountError> {
        // Try to get the validator. It might have been deleted.
        if let Some(mut validator) = store.get_validator(validator_address) {
            // Validator exists, update it.
            validator.total_stake += value;

            if validator.is_active() {
                self.active_validators
                    .insert(validator_address.clone(), validator.total_stake);
            }

            // Update the validator entry.
            store.put_validator(validator_address, validator);

            return Ok(());
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.remaining_stake += value;

            // Update the tombstone entry.
            store.put_tombstone(validator_address, tombstone);

            return Ok(());
        }

        // Neither validator nor tombstone exist, this is an error.
        panic!("inconsistent contract state");
    }

    /// Removes `value` coins from a given validator's total stake.
    fn remove_stake_from_validator(
        &mut self,
        store: &mut StakingContractStoreWrite,
        validator_address: &Address,
        value: Coin,
    ) -> Result<(), AccountError> {
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

            return Ok(());
        }

        // Validator doesn't exist, check for tombstone.
        if let Some(mut tombstone) = store.get_tombstone(validator_address) {
            // Tombstone exists, update it.
            tombstone.remaining_stake -= value;

            // Update the tombstone entry.
            store.put_tombstone(validator_address, tombstone);

            return Ok(());
        }

        // Neither validator nor tombstone exist, this is an error.
        panic!("inconsistent contract state");
    }
}
