use nimiq_keys::Address;
use nimiq_primitives::{
    account::{AccountError, AccountType},
    coin::Coin,
};
use nimiq_serde::Deserialize;
use nimiq_transaction::{
    account::staking_contract::{IncomingStakingTransactionData, OutgoingStakingTransactionData},
    inherent::Inherent,
    SignatureProof, Transaction,
};

use crate::{
    account::staking_contract::{
        receipts::PenalizeReceipt,
        store::{
            StakingContractStoreRead, StakingContractStoreReadOps, StakingContractStoreReadOpsExt,
            StakingContractStoreWrite,
        },
        StakingContract,
    },
    data_store::{DataStoreRead, DataStoreWrite},
    interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction},
    reserved_balance::ReservedBalance,
    Account, AccountPruningInteraction, AccountReceipt, BlockState, InherentLogger, JailReceipt,
    JailValidatorReceipt, Log, TransactionLog,
};

impl AccountTransactionInteraction for StakingContract {
    fn create_new_contract(
        _transaction: &Transaction,
        _initial_balance: Coin,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<Account, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_new_contract(
        &mut self,
        _transaction: &Transaction,
        _block_state: &BlockState,
        _data_store: DataStoreWrite,
        _tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        mut data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
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
                    None,
                    None,
                    false,
                    tx_logger,
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
                    tx_logger,
                )
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
                    tx_logger,
                )
                .map(|_| None)
            }
            IncomingStakingTransactionData::ReactivateValidator {
                validator_address,
                proof,
            } => {
                // Get the signer's address from the proof.
                let signer = proof.compute_signer();

                self.reactivate_validator(
                    &mut store,
                    &validator_address,
                    &signer,
                    block_state.number,
                    tx_logger,
                )
                .map(|receipt| Some(receipt.into()))
            }
            IncomingStakingTransactionData::RetireValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                self.retire_validator(
                    &mut store,
                    &validator_address,
                    block_state.number,
                    tx_logger,
                )
                .map(|receipt| Some(receipt.into()))
            }
            IncomingStakingTransactionData::CreateStaker { delegation, proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                self.create_staker(
                    &mut store,
                    &staker_address,
                    transaction.value,
                    delegation,
                    Coin::ZERO,
                    None,
                    tx_logger,
                )
                .map(|_| None)
            }
            IncomingStakingTransactionData::AddStake { staker_address } => self
                .add_stake(&mut store, &staker_address, transaction.value, tx_logger)
                .map(|_| None),
            IncomingStakingTransactionData::UpdateStaker {
                new_delegation,
                reactivate_all_stake,
                proof,
            } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                self.update_staker(
                    &mut store,
                    &staker_address,
                    new_delegation,
                    reactivate_all_stake,
                    block_state.number,
                    tx_logger,
                )
                .map(|receipt| Some(receipt.into()))
            }
            IncomingStakingTransactionData::SetActiveStake {
                new_active_balance,
                proof,
            } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                self.set_active_stake(
                    &mut store,
                    &staker_address,
                    new_active_balance,
                    block_state.number,
                    tx_logger,
                )
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
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction data.
        let data = IncomingStakingTransactionData::parse(transaction)?;

        match data {
            IncomingStakingTransactionData::CreateValidator { proof, .. } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                self.revert_create_validator(
                    &mut store,
                    &validator_address,
                    transaction.value,
                    tx_logger,
                )
            }
            IncomingStakingTransactionData::UpdateValidator { proof, .. } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_update_validator(&mut store, &validator_address, receipt, tx_logger)
            }
            IncomingStakingTransactionData::DeactivateValidator {
                validator_address, ..
            } => self.revert_deactivate_validator(&mut store, &validator_address, tx_logger),
            IncomingStakingTransactionData::ReactivateValidator {
                validator_address, ..
            } => {
                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_reactivate_validator(&mut store, &validator_address, receipt, tx_logger)
            }
            IncomingStakingTransactionData::RetireValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_retire_validator(&mut store, &validator_address, receipt, tx_logger)
            }
            IncomingStakingTransactionData::CreateStaker { proof, .. } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                self.revert_create_staker(&mut store, &staker_address, transaction.value, tx_logger)
            }
            IncomingStakingTransactionData::AddStake { staker_address } => {
                self.revert_add_stake(&mut store, &staker_address, transaction.value, tx_logger)
            }
            IncomingStakingTransactionData::UpdateStaker { proof, .. } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_update_staker(&mut store, &staker_address, receipt, tx_logger)
            }
            IncomingStakingTransactionData::SetActiveStake {
                new_active_balance,
                proof,
            } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_set_active_stake(
                    &mut store,
                    &staker_address,
                    new_active_balance,
                    receipt,
                    tx_logger,
                )
            }
        }
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        mut data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction proof.
        let data = OutgoingStakingTransactionData::parse(transaction)?;
        let proof: SignatureProof = Deserialize::deserialize_from_vec(&transaction.proof[..])?;

        tx_logger.push_log(Log::pay_fee_log(transaction));
        tx_logger.push_log(Log::transfer_log(transaction));

        match data {
            OutgoingStakingTransactionData::DeleteValidator => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                self.delete_validator(
                    &mut store,
                    &validator_address,
                    block_state.number,
                    transaction.total_value(),
                    tx_logger,
                )
                .map(|receipt| Some(receipt.into()))
            }
            OutgoingStakingTransactionData::RemoveStake => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                self.remove_stake(
                    &mut store,
                    &staker_address,
                    transaction.total_value(),
                    block_state.number,
                    tx_logger,
                )
                .map(|receipt| Some(receipt.into()))
            }
        }
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        mut data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction data.
        let data = OutgoingStakingTransactionData::parse(transaction)?;
        let proof: SignatureProof = Deserialize::deserialize_from_vec(&transaction.proof[..])?;

        let result = match data {
            OutgoingStakingTransactionData::DeleteValidator => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_delete_validator(
                    &mut store,
                    &validator_address,
                    transaction.total_value(),
                    receipt,
                    tx_logger,
                )
            }
            OutgoingStakingTransactionData::RemoveStake => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_remove_stake(
                    &mut store,
                    &staker_address,
                    transaction.total_value(),
                    receipt,
                    tx_logger,
                )
            }
        };

        tx_logger.push_log(Log::transfer_log(transaction));
        tx_logger.push_log(Log::pay_fee_log(transaction));

        result
    }

    fn commit_failed_transaction(
        &mut self,
        transaction: &Transaction,
        block_state: &BlockState,
        mut data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction proof.
        let data = OutgoingStakingTransactionData::parse(transaction)?;
        let proof: SignatureProof = Deserialize::deserialize_from_vec(&transaction.proof[..])?;

        tx_logger.push_log(Log::pay_fee_log(transaction));

        let receipt = match data {
            // In the case of a failed Delete Validator we will:
            // 1. Pay the fee from the validator deposit
            // 2. If the deposit reaches 0, we delete the validator
            OutgoingStakingTransactionData::DeleteValidator => {
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
                        tx_logger,
                    )?;

                    Some(receipt.into())
                } else {
                    // Update the validator deposit and total_stake.
                    validator.deposit = new_deposit;
                    validator.total_stake -= transaction.fee;

                    // Update the validator entry.
                    store.put_validator(&validator_address, validator);

                    // Update our balance.
                    self.balance -= transaction.fee;

                    None
                };

                tx_logger.push_log(Log::ValidatorFeeDeduction {
                    validator_address,
                    fee: transaction.fee,
                });

                receipt
            }
            OutgoingStakingTransactionData::RemoveStake => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                // This is similar to an remove_stake operation except that what we deduct only the fee from the stake.
                // We do not want the fee payment to be displayed as a successful unstake in the block logs,
                // which is why we pass an empty logger.
                let receipt = self
                    .remove_stake(
                        &mut store,
                        &staker_address,
                        transaction.fee,
                        block_state.number,
                        &mut TransactionLog::empty(),
                    )
                    .map(|receipt| Some(receipt.into()))?;

                tx_logger.push_log(Log::StakerFeeDeduction {
                    staker_address,
                    fee: transaction.fee,
                });

                receipt
            }
        };

        Ok(receipt)
    }

    fn revert_failed_transaction(
        &mut self,
        transaction: &Transaction,
        _block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        mut data_store: DataStoreWrite,
        tx_logger: &mut TransactionLog,
    ) -> Result<(), AccountError> {
        let mut store = StakingContractStoreWrite::new(&mut data_store);

        // Parse transaction data.
        let data = OutgoingStakingTransactionData::parse(transaction)?;
        let proof: SignatureProof = Deserialize::deserialize_from_vec(&transaction.proof[..])?;

        let result = match data {
            OutgoingStakingTransactionData::DeleteValidator => {
                let validator_address = proof.compute_signer();
                tx_logger.push_log(Log::ValidatorFeeDeduction {
                    validator_address: validator_address.clone(),
                    fee: transaction.fee,
                });

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
                            tx_logger,
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
            OutgoingStakingTransactionData::RemoveStake => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();
                tx_logger.push_log(Log::StakerFeeDeduction {
                    staker_address: staker_address.clone(),
                    fee: transaction.fee,
                });

                let receipt = receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.revert_remove_stake(
                    &mut store,
                    &staker_address,
                    transaction.fee,
                    receipt,
                    &mut TransactionLog::empty(),
                )
            }
        };

        tx_logger.push_log(Log::pay_fee_log(transaction));

        result
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
        let data = OutgoingStakingTransactionData::parse(transaction)?;
        let proof: SignatureProof = Deserialize::deserialize_from_vec(&transaction.proof[..])?;

        match data {
            OutgoingStakingTransactionData::DeleteValidator => {
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
            OutgoingStakingTransactionData::RemoveStake => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let staker = store.expect_staker(&staker_address)?;

                self.can_remove_stake(&store, &staker, block_state.number)?;

                reserved_balance.reserve_for(
                    &staker_address,
                    staker.inactive_balance,
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
        let data = OutgoingStakingTransactionData::parse(transaction)?;
        let proof: SignatureProof = Deserialize::deserialize_from_vec(&transaction.proof[..])?;

        match data {
            OutgoingStakingTransactionData::DeleteValidator => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                reserved_balance.release_for(&validator_address, transaction.total_value());
            }
            OutgoingStakingTransactionData::RemoveStake => {
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
        inherent_logger: &mut InherentLogger,
    ) -> Result<Option<AccountReceipt>, AccountError> {
        match inherent {
            Inherent::Jail {
                jailed_validator,
                new_epoch_slot_range,
            } => {
                // Jail validator
                let mut store = StakingContractStoreWrite::new(&mut data_store);
                let mut tx_logger = TransactionLog::empty();
                let receipt = self.jail_validator(
                    &mut store,
                    &jailed_validator.validator_address,
                    block_state.number,
                    &mut tx_logger,
                )?;
                inherent_logger.push_tx_logger(tx_logger);

                let newly_deactivated = receipt.newly_deactivated;
                let newly_jailed = receipt.old_jailed_from.is_none();

                // Register the validator slots as punished
                let (old_previous_batch_punished_slots, old_current_batch_punished_slots) =
                    self.punished_slots.register_jail(
                        jailed_validator,
                        block_state.number,
                        new_epoch_slot_range.clone(),
                    );

                inherent_logger.push_log(Log::Jail {
                    validator_address: jailed_validator.validator_address.clone(),
                    event_block: jailed_validator.offense_event_block,
                    newly_jailed,
                });

                Ok(Some(
                    JailReceipt {
                        newly_deactivated,
                        old_previous_batch_punished_slots,
                        old_current_batch_punished_slots,
                        old_jailed_from: receipt.old_jailed_from,
                    }
                    .into(),
                ))
            }
            Inherent::Penalize { slot } => {
                // Check that the penalized validator does exist.
                let mut store = StakingContractStoreWrite::new(&mut data_store);
                let validator = store.expect_validator(&slot.validator_address)?;

                // Deactivate validator
                let newly_deactivated = validator.is_active();
                if newly_deactivated {
                    let mut tx_logger = TransactionLog::empty();
                    self.deactivate_validator(
                        &mut store,
                        &slot.validator_address,
                        &Address::from(&validator.signing_key),
                        block_state.number,
                        &mut tx_logger,
                    )?;
                    inherent_logger.push_tx_logger(tx_logger);
                }

                // Penalize the validator
                let (newly_punished_previous_batch, newly_punished_current_batch) = self
                    .punished_slots
                    .register_penalty(slot, block_state.number);

                inherent_logger.push_log(Log::Penalize {
                    validator_address: slot.validator_address.clone(),
                    offense_event_block: slot.offense_event_block,
                    slot: slot.slot,
                    newly_deactivated,
                });

                Ok(Some(
                    PenalizeReceipt {
                        newly_deactivated,
                        newly_punished_previous_batch,
                        newly_punished_current_batch,
                    }
                    .into(),
                ))
            }
            Inherent::FinalizeBatch => {
                // Clear the lost rewards set.
                self.punished_slots
                    .finalize_batch(block_state.number, &self.active_validators);

                // Since finalized batches cannot be reverted, we don't need any receipts.
                Ok(None)
            }
            Inherent::FinalizeEpoch => {
                // Since finalized epochs cannot be reverted, we don't need any receipts.
                Ok(None)
            }
            Inherent::Reward { .. } => Err(AccountError::InvalidForTarget),
        }
    }

    fn revert_inherent(
        &mut self,
        inherent: &Inherent,
        _block_state: &BlockState,
        receipt: Option<AccountReceipt>,
        mut data_store: DataStoreWrite,
        inherent_logger: &mut InherentLogger,
    ) -> Result<(), AccountError> {
        match inherent {
            Inherent::Jail {
                jailed_validator, ..
            } => {
                let receipt: JailReceipt =
                    receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;
                let newly_jailed = receipt.old_jailed_from.is_none();
                let jail_receipt = JailValidatorReceipt::from(&receipt);

                self.punished_slots.revert_register_jail(
                    jailed_validator,
                    receipt.old_previous_batch_punished_slots,
                    receipt.old_current_batch_punished_slots,
                );

                inherent_logger.push_log(Log::Jail {
                    validator_address: jailed_validator.validator_address.clone(),
                    event_block: jailed_validator.offense_event_block,
                    newly_jailed,
                });

                let mut tx_logger = TransactionLog::empty();
                self.revert_jail_validator(
                    &mut StakingContractStoreWrite::new(&mut data_store),
                    &jailed_validator.validator_address,
                    jail_receipt,
                    &mut tx_logger,
                )?;
                inherent_logger.push_tx_logger(tx_logger);

                Ok(())
            }
            Inherent::Penalize { slot } => {
                let receipt: PenalizeReceipt =
                    receipt.ok_or(AccountError::InvalidReceipt)?.try_into()?;

                self.punished_slots.revert_register_penalty(
                    slot,
                    receipt.newly_punished_previous_batch,
                    receipt.newly_punished_current_batch,
                );

                inherent_logger.push_log(Log::Penalize {
                    validator_address: slot.validator_address.clone(),
                    offense_event_block: slot.offense_event_block,
                    slot: slot.slot,
                    newly_deactivated: receipt.newly_deactivated,
                });

                if receipt.newly_deactivated {
                    let mut tx_logger = TransactionLog::empty();
                    self.revert_deactivate_validator(
                        &mut StakingContractStoreWrite::new(&mut data_store),
                        &slot.validator_address,
                        &mut tx_logger,
                    )?;

                    inherent_logger.push_tx_logger(tx_logger);
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
