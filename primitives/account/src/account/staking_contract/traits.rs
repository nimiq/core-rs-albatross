use std::collections::BTreeSet;
use std::mem;

use nimiq_primitives::account::AccountType;
use nimiq_primitives::{account::AccountError, coin::Coin, policy::Policy};
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};
use nimiq_transaction::{inherent::Inherent, Transaction};

use crate::account::staking_contract::store::{
    StakingContractStoreRead, StakingContractStoreReadOps, StakingContractStoreReadOpsExt,
    StakingContractStoreWrite,
};
use crate::reserved_balance::ReservedBalance;
use crate::{
    account::staking_contract::{receipts::SlashReceipt, StakingContract},
    data_store::{DataStoreRead, DataStoreWrite},
    interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction},
    Account, AccountPruningInteraction, AccountReceipt, BlockState,
};

impl AccountTransactionInteraction for StakingContract {
    fn create_new_contract(
        _transaction: &Transaction,
        _initial_balance: Coin,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_new_contract(
        &mut self,
        _transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        mut data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction data.
        let data = IncomingStakingTransactionData::parse(transaction)?;

        match data {
            IncomingStakingTransactionData::CreateValidator {
                signing_key,
                voting_key,
                reward_address,
                signal_data,
                proof,
                ..
            } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                // XXX Already checked during intrinsic transaction verification.
                // // Get the deposit value.
                // let deposit = Coin::from_u64_unchecked(policy::VALIDATOR_DEPOSIT);
                //
                // // Verify the transaction was formed properly
                // if transaction.value != deposit {
                //     return Err(AccountError::InvalidCoinValue);
                // }

                self.create_validator(
                    &mut store,
                    &validator_address,
                    signing_key,
                    voting_key,
                    reward_address,
                    signal_data,
                    transaction.value,
                )
                .map(|_| None)
            }
            IncomingStakingTransactionData::UpdateValidator {
                new_signing_key,
                new_voting_key,
                new_reward_address,
                new_signal_data,
                proof,
                ..
            } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                self.update_validator(
                    &mut store,
                    &validator_address,
                    new_signing_key,
                    new_voting_key,
                    new_reward_address,
                    new_signal_data,
                )
                .map(|receipt| Some(receipt.into()))
            }
            IncomingStakingTransactionData::UnparkValidator {
                validator_address,
                proof,
            } => {
                // Get the signer's address from the proof.
                let signer = proof.compute_signer();

                self.unpark_validator(&mut store, &validator_address, &signer)
                    .map(|receipt| Some(receipt.into()))
            }
            IncomingStakingTransactionData::DeactivateValidator {
                validator_address,
                proof,
            } => {
                // Get the signer's address from the proof.
                let signer = proof.compute_signer();

                self.deactivate_validator(
                    &mut store,
                    &validator_address,
                    &signer,
                    block_state.number,
                )
                .map(|receipt| Some(receipt.into()))
            }
            IncomingStakingTransactionData::ReactivateValidator {
                validator_address,
                proof,
            } => {
                // Get the signer's address from the proof.
                let signer = proof.compute_signer();

                self.reactivate_validator(&mut store, &validator_address, &signer)
                    .map(|receipt| Some(receipt.into()))
            }
            IncomingStakingTransactionData::RetireValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                self.retire_validator(&mut store, &validator_address, block_state.number)
                    .map(|receipt| Some(receipt.into()))
            }
            IncomingStakingTransactionData::CreateStaker { delegation, proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                self.create_staker(&mut store, &staker_address, transaction.value, delegation)
                    .map(|_| None)
            }
            IncomingStakingTransactionData::AddStake { staker_address } => self
                .add_stake(&mut store, &staker_address, transaction.value)
                .map(|_| None),
            IncomingStakingTransactionData::UpdateStaker {
                new_delegation,
                proof,
            } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                self.update_staker(&mut store, &staker_address, new_delegation)
                    .map(|receipt| Some(receipt.into()))
            }
        }
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        mut data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction data.
        let data = IncomingStakingTransactionData::parse(transaction)?;

        match data {
            IncomingStakingTransactionData::CreateValidator { proof, .. } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                self.revert_create_validator(&mut store, &validator_address, transaction.value)
            }
            IncomingStakingTransactionData::UpdateValidator { proof, .. } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_update_validator(&mut store, &validator_address, receipt)
            }
            IncomingStakingTransactionData::UnparkValidator {
                validator_address, ..
            } => {
                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_unpark_validator(&mut store, &validator_address, receipt)
            }
            IncomingStakingTransactionData::DeactivateValidator {
                validator_address, ..
            } => {
                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_deactivate_validator(&mut store, &validator_address, receipt)
            }
            IncomingStakingTransactionData::ReactivateValidator {
                validator_address, ..
            } => {
                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_reactivate_validator(&mut store, &validator_address, receipt)
            }
            IncomingStakingTransactionData::RetireValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_retire_validator(&mut store, &validator_address, receipt)
            }
            IncomingStakingTransactionData::CreateStaker { proof, .. } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                self.revert_create_staker(&mut store, &staker_address, transaction.value)
            }
            IncomingStakingTransactionData::AddStake { staker_address } => {
                self.revert_add_stake(&mut store, &staker_address, transaction.value)
            }
            IncomingStakingTransactionData::UpdateStaker { proof, .. } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_update_staker(&mut store, &staker_address, receipt)
            }
        }
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        mut data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction proof.
        let proof = OutgoingStakingTransactionProof::parse(transaction)?;

        match proof {
            OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                self.delete_validator(
                    &mut store,
                    &validator_address,
                    block_state.number,
                    transaction.total_value(),
                )
                .map(|receipt| Some(receipt.into()))
            }
            OutgoingStakingTransactionProof::RemoveStake { proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                self.remove_stake(&mut store, &staker_address, transaction.total_value())
                    .map(|receipt| receipt.map(|receipt| receipt.into()))
            }
        }

        // // Ordering matters here for testing purposes. The vec will be very small, therefore the performance hit is irrelevant.
        // acc_info.logs.insert(
        //     0,
        //     Log::Transfer {
        //         from: transaction.sender.clone(),
        //         to: transaction.recipient.clone(),
        //         amount: transaction.value,
        //     },
        // );
        // acc_info.logs.insert(
        //     0,
        //     Log::PayFee {
        //         from: transaction.sender.clone(),
        //         fee: transaction.fee,
        //     },
        // );
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        mut data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction data.
        let data = OutgoingStakingTransactionProof::parse(transaction)?;

        match data {
            OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_delete_validator(
                    &mut store,
                    &validator_address,
                    transaction.total_value(),
                    receipt,
                )
            }
            OutgoingStakingTransactionProof::RemoveStake { proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let receipt = match receipt {
                    Some(receipt) => Some(receipt.try_into()?),
                    None => None,
                };

                self.revert_remove_stake(
                    &mut store,
                    &staker_address,
                    transaction.total_value(),
                    receipt,
                )
            }
        }
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        mut data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction proof.
        let data = OutgoingStakingTransactionProof::parse(transaction)?;

        match data {
            // In the case of a failed Delete Validator we will:
            // 1. Pay the fee from the validator deposit
            // 2. If the deposit reaches 0, we delete the validator
            OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                let validator_address = proof.compute_signer();

                let mut validator = store.expect_validator(&validator_address)?;

                self.can_delete_validator(&validator, block_state.number)?;

                let new_deposit = validator.deposit.safe_sub(transaction.fee)?;

                // Delete the validator if the deposit reaches zero.
                let receipt = if new_deposit.is_zero() {
                    let receipt = self.delete_validator(
                        &mut store,
                        &validator_address,
                        block_state.number,
                        validator.deposit,
                    )?;

                    Some(receipt.into())
                } else {
                    // Update the validator deposit and total_stake.
                    validator.deposit = new_deposit;
                    validator.total_stake -= transaction.fee;

                    // Update the validator entry.
                    store.put_validator(&validator_address, validator);

                    None
                };

                // Update our balance.
                self.balance -= transaction.fee;

                Ok(receipt)
            }
            OutgoingStakingTransactionProof::RemoveStake { proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                // This is similar to an remove_stake operation except that what we deduct only the fee from the stake.
                self.remove_stake(&mut store, &staker_address, transaction.fee)
                    .map(|receipt| receipt.map(|receipt| receipt.into()))
            }
        }

        // acc_info.logs.insert(
        //     0,
        //     Log::PayFee {
        //         from: transaction.sender.clone(),
        //         fee: transaction.fee,
        //     },
        // );
    }

    fn revert_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        mut data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction data.
        let data = OutgoingStakingTransactionProof::parse(transaction)?;

        match data {
            OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                let validator_address = proof.compute_signer();

                // Get or restore validator.
                let mut validator = {
                    if let Some(validator) = store.get_validator(&validator_address) {
                        validator
                    } else if let Some(receipt) = receipt {
                        self.revert_delete_validator(
                            &mut store,
                            &validator_address,
                            Coin::ZERO,
                            receipt.try_into()?,
                        )?;

                        store
                            .get_validator(&validator_address)
                            .expect("validator should be restored")
                    } else {
                        return Err(AccountError::InvalidReceipt);
                    }
                };

                // Update the validator's deposit and total_stake.
                validator.deposit += transaction.fee;
                validator.total_stake += transaction.fee;

                // Update the validator entry.
                store.put_validator(&validator_address, validator);

                // Update our balance.
                self.balance += transaction.fee;

                Ok(())
            }
            OutgoingStakingTransactionProof::RemoveStake { proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let receipt = match receipt {
                    Some(receipt) => Some(receipt.try_into()?),
                    None => None,
                };

                self.revert_remove_stake(&mut store, &staker_address, transaction.fee, receipt)
            }
        }

        // acc_info.logs.insert(
        //     0,
        //     Log::PayFee {
        //         from: transaction.sender.clone(),
        //         fee: transaction.fee,
        //     },
        // );
    }

    fn reserve_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        block_state: &BlockState,
        data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        let store = StakingContractStoreRead::new(&data_store);

        // Parse transaction proof.
        let proof = OutgoingStakingTransactionProof::parse(transaction)?;

        match proof {
            OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                // Fetch the validator.
                let validator = store.expect_validator(&validator_address)?;

                // Verify that the validator can actually be deleted.
                self.can_delete_validator(&validator, block_state.number)?;

                reserved_balance.reserve_for(
                    &validator_address,
                    validator.deposit,
                    transaction.total_value(),
                )
            }
            OutgoingStakingTransactionProof::RemoveStake { proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let staker = store.expect_staker(&staker_address)?;

                reserved_balance.reserve_for(
                    &staker_address,
                    staker.balance,
                    transaction.total_value(),
                )
            }
        }
    }

    fn release_balance(
        &self,
        transaction: &Transaction,
        reserved_balance: &mut ReservedBalance,
        _data_store: DataStoreRead,
    ) -> Result<(), AccountError> {
        // Parse transaction proof.
        let proof = OutgoingStakingTransactionProof::parse(transaction)?;

        match proof {
            OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                reserved_balance.release_for(&validator_address, transaction.total_value());
            }
            OutgoingStakingTransactionProof::RemoveStake { proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                reserved_balance.release_for(&staker_address, transaction.total_value())
            }
        }

        Ok(())
    }
}

impl AccountInherentInteraction for StakingContract {
    fn commit_inherent(
        &mut self,
        inherent: &Inherent,
        block_state: &BlockState,
        mut data_store: DataStoreWrite,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        match inherent {
            Inherent::Slash { slot } => {
                // Check that the slashed validator does exist.
                let store = StakingContractStoreWrite::new(&mut data_store);
                store.expect_validator(&slot.validator_address)?;

                // Add the validator address to the parked set.
                // TODO: The inherent might have originated from a fork proof for the previous epoch.
                //  Right now, we don't care and start the parking period in the epoch the proof has been submitted.
                let newly_parked = self.parked_set.insert(slot.validator_address.clone());

                // Fork proof from previous epoch should affect:
                // - previous_lost_rewards
                // - previous_disabled_slots (not needed, because it's redundant with the lost rewards)
                // Fork proof from current epoch, but previous batch should affect:
                // - previous_lost_rewards
                // - current_disabled_slots
                // All others:
                // - current_lost_rewards
                // - current_disabled_slots
                let newly_disabled;
                let newly_lost_rewards;

                if Policy::epoch_at(slot.event_block) < Policy::epoch_at(block_state.number) {
                    newly_lost_rewards = !self.previous_lost_rewards.contains(slot.slot as usize);

                    self.previous_lost_rewards.insert(slot.slot as usize);

                    newly_disabled = false;
                } else if Policy::batch_at(slot.event_block) < Policy::batch_at(block_state.number)
                {
                    newly_lost_rewards = !self.previous_lost_rewards.contains(slot.slot as usize);

                    self.previous_lost_rewards.insert(slot.slot as usize);

                    newly_disabled = self
                        .current_disabled_slots
                        .entry(slot.validator_address.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(slot.slot);
                } else {
                    newly_lost_rewards = !self.current_lost_rewards.contains(slot.slot as usize);

                    self.current_lost_rewards.insert(slot.slot as usize);

                    newly_disabled = self
                        .current_disabled_slots
                        .entry(slot.validator_address.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(slot.slot);
                }

                // if newly_lost_rewards {
                //     logs.push(Log::Slash {
                //         validator_address: slot.validator_address.clone(),
                //         event_block: slot.event_block,
                //         slot: slot.slot,
                //         newly_disabled,
                //     });
                // }
                // if newly_parked {
                //     logs.push(Log::Park {
                //         validator_address: slot.validator_address,
                //         event_block: block_height,
                //     });
                // }

                Ok(Some(
                    SlashReceipt {
                        newly_parked,
                        newly_disabled,
                        newly_lost_rewards,
                    }
                    .into(),
                ))
            }
            Inherent::FinalizeBatch => {
                // Clear the lost rewards set.
                self.previous_lost_rewards = mem::take(&mut self.current_lost_rewards);

                // Since finalized batches cannot be reverted, we don't need any receipts.
                Ok(None)
            }
            Inherent::FinalizeEpoch => {
                // Clear the lost rewards set.
                self.previous_lost_rewards = mem::take(&mut self.current_lost_rewards);

                // Parking set and disabled slots are cleared on epoch changes.
                // But first, retire all validators that have been parked this epoch.
                let mut store = StakingContractStoreWrite::new(&mut data_store);
                for validator_address in &self.parked_set {
                    // Get the validator and update it.
                    let mut validator = store.expect_validator(validator_address)?;
                    validator.inactive_since = Some(block_state.number);
                    store.put_validator(validator_address, validator);

                    // Update the staking contract.
                    self.active_validators.remove(validator_address);

                    // logs.push(Log::DeactivateValidator {
                    //     validator_address: validator_address.clone(),
                    // });
                }

                // Now we clear the parking set.
                self.parked_set = BTreeSet::new();

                // And the disabled slots.
                // Optimization: We actually only need the old slots for the first batch of the epoch.
                self.previous_disabled_slots = mem::take(&mut self.current_disabled_slots);

                // Since finalized epochs cannot be reverted, we don't need any receipts.
                Ok(None)
            }
            Inherent::Reward { .. } => Err(AccountError::InvalidForTarget),
        }
    }

    fn revert_inherent(
        &mut self,
        inherent: &Inherent,
        block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<(), AccountError> {
        match inherent {
            Inherent::Slash { slot } => {
                let receipt: SlashReceipt =
                    receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                // Only remove if it was not already slashed.
                if receipt.newly_parked {
                    let has_been_removed = self.parked_set.remove(&slot.validator_address);

                    if !has_been_removed {
                        return Err(AccountError::InvalidInherent);
                    }

                    // logs.push(Log::Park {
                    //     validator_address: slot.validator_address.clone(),
                    //     event_block: block_height,
                    // });
                }

                // Fork proof from previous epoch should affect:
                // - previous_lost_rewards
                // - previous_disabled_slots (not needed, because it's redundant with the lost rewards)
                // Fork proof from current epoch, but previous batch should affect:
                // - previous_lost_rewards
                // - current_disabled_slots
                // All others:
                // - current_lost_rewards
                // - current_disabled_slots
                if receipt.newly_disabled {
                    if Policy::epoch_at(slot.event_block) < Policy::epoch_at(block_state.number) {
                        // Nothing to do.
                    } else {
                        let is_empty = {
                            let entry = self
                                .current_disabled_slots
                                .get_mut(&slot.validator_address)
                                .unwrap();
                            entry.remove(&slot.slot);
                            entry.is_empty()
                        };
                        if is_empty {
                            self.current_disabled_slots.remove(&slot.validator_address);
                        }
                    }
                }
                if receipt.newly_lost_rewards {
                    if Policy::epoch_at(slot.event_block) < Policy::epoch_at(block_state.number)
                        || Policy::batch_at(slot.event_block) < Policy::batch_at(block_state.number)
                    {
                        self.previous_lost_rewards.remove(slot.slot as usize);
                    } else {
                        self.current_lost_rewards.remove(slot.slot as usize);
                    }

                    // Ordering matters here for testing purposes. The vec will be very small, therefore the performance hit is irrelevant.
                    // logs.insert(
                    //     0,
                    //     Log::Slash {
                    //         validator_address: slot.validator_address,
                    //         event_block: slot.event_block,
                    //         slot: slot.slot,
                    //         newly_disabled: true,
                    //     },
                    // );
                }

                Ok(())
            }
            Inherent::FinalizeBatch | Inherent::FinalizeEpoch => {
                // We should not be able to revert finalized epochs or batches!
                Err(AccountError::InvalidForTarget)
            }
            Inherent::Reward { .. } => Err(AccountError::InvalidForTarget),
        }
    }
}

impl AccountPruningInteraction for StakingContract {
    fn can_be_pruned(&self) -> bool {
        false
    }

    fn prune(self, _data_store: DataStoreRead) -> Option<AccountReceipt> {
        unreachable!()
    }

    fn restore(
        _ty: AccountType,
        _pruned_account: Option<&AccountReceipt>,
        _data_store: DataStoreWrite,
    ) -> Result<Account, AccountError> {
        unreachable!()
    }
}
