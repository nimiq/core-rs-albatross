use std::collections::{BTreeMap, BTreeSet};

use beserial::{Deserialize, Serialize};
use nimiq_collections::BitSet;
use nimiq_database::WriteTransaction;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;
use nimiq_primitives::slots::SlashedSlot;
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof,
};
use nimiq_transaction::Transaction;

use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::staking_contract::receipts::DropValidatorReceipt;
use crate::staking_contract::SlashReceipt;
use crate::{Account, AccountError, AccountsTree, Inherent, InherentType, StakingContract};

/// We need to distinguish three types of transactions:
/// TODO: Should invalid incoming transactions just be no-ops?
/// 1. Incoming transactions, which include:
///     - Validator
///         * Create
///         * Update
///         * Retire
///         * Re-activate
///         * Unpark
///     - Staker
///         * Stake
///     The type of transaction is given in the data field.
/// 2. Outgoing transactions, which include:
///     - Validator
///         * Drop
///     - Staker
///         * Unstake
///     The type of transaction is given in the proof field.
/// 3. Self transactions, which include:
///     - Staker
///         * Retire
///         * Re-activate
///     The type of transaction is given in the data field.
impl AccountTransactionInteraction for StakingContract {
    fn create(
        _accounts_tree: &AccountsTree,
        _db_txn: &mut WriteTransaction,
        _balance: Coin,
        _transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        let mut receipt = None;

        // Parse transaction.
        let data = IncomingStakingTransactionData::parse(transaction)?;

        match data {
            IncomingStakingTransactionData::CreateValidator {
                warm_key,
                validator_key,
                reward_address,
                signal_data,
                proof: signature,
                ..
            } => {
                let validator_address = signature.compute_signer();

                StakingContract::create_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    warm_key,
                    validator_key,
                    reward_address,
                    signal_data,
                )?;
            }
            IncomingStakingTransactionData::UpdateValidator {
                new_warm_key,
                new_validator_key,
                new_reward_address,
                new_signal_data,
                proof: signature,
                ..
            } => {
                let validator_address = signature.compute_signer();

                receipt = Some(
                    StakingContract::update_validator(
                        accounts_tree,
                        db_txn,
                        &validator_address,
                        new_warm_key,
                        new_validator_key,
                        new_reward_address,
                        new_signal_data,
                    )?
                    .serialize_to_vec(),
                )
            }
            IncomingStakingTransactionData::RetireValidator { proof: signature } => {
                let validator_address = signature.compute_signer();

                receipt = Some(
                    StakingContract::retire_validator(
                        accounts_tree,
                        db_txn,
                        &validator_address,
                        block_height,
                    )?
                    .serialize_to_vec(),
                );
            }
            IncomingStakingTransactionData::ReactivateValidator { proof: signature } => {
                let validator_address = signature.compute_signer();

                receipt = Some(
                    StakingContract::reactivate_validator(
                        accounts_tree,
                        db_txn,
                        &validator_address,
                    )?
                    .serialize_to_vec(),
                );
            }
            IncomingStakingTransactionData::UnparkValidator { proof: signature } => {
                let validator_address = signature.compute_signer();

                receipt = Some(
                    StakingContract::unpark_validator(accounts_tree, db_txn, &validator_address)?
                        .serialize_to_vec(),
                );
            }
            IncomingStakingTransactionData::CreateStaker {
                delegation,
                proof: signature,
            } => {
                let staker_address = signature.compute_signer();

                StakingContract::create_staker(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    transaction.value,
                    delegation,
                )?;
            }
            IncomingStakingTransactionData::Stake { staker_address } => {
                StakingContract::stake(accounts_tree, db_txn, &staker_address, transaction.value)?;
            }
            IncomingStakingTransactionData::UpdateStaker {
                new_delegation,
                proof: signature,
            } => {
                let staker_address = signature.compute_signer();

                receipt = Some(
                    StakingContract::update_staker(
                        accounts_tree,
                        db_txn,
                        &staker_address,
                        new_delegation,
                    )?
                    .serialize_to_vec(),
                );
            }
            IncomingStakingTransactionData::RetireStaker {
                value,
                proof: signature,
            } => {
                let staker_address = signature.compute_signer();

                receipt = Some(
                    StakingContract::retire_staker(
                        accounts_tree,
                        db_txn,
                        &staker_address,
                        value,
                        block_height,
                    )?
                    .serialize_to_vec(),
                );
            }
            IncomingStakingTransactionData::ReactivateStaker {
                value,
                proof: signature,
            } => {
                let staker_address = signature.compute_signer();

                StakingContract::reactivate_staker(accounts_tree, db_txn, &staker_address, value)?;
            }
        }

        Ok(receipt)
    }

    fn revert_incoming_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        let data: IncomingStakingTransactionData =
            Deserialize::deserialize(&mut &transaction.data[..])?;

        match data {
            IncomingStakingTransactionData::CreateValidator {
                proof: signature, ..
            } => {
                let validator_address = signature.compute_signer();

                StakingContract::revert_create_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                )?;
            }
            IncomingStakingTransactionData::UpdateValidator {
                proof: signature, ..
            } => {
                let validator_address = signature.compute_signer();

                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                StakingContract::revert_update_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    receipt,
                )?;
            }
            IncomingStakingTransactionData::RetireValidator {
                proof: signature, ..
            } => {
                let validator_address = signature.compute_signer();

                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                StakingContract::revert_retire_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    receipt,
                )?;
            }
            IncomingStakingTransactionData::ReactivateValidator {
                proof: signature, ..
            } => {
                let validator_address = signature.compute_signer();

                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                StakingContract::revert_reactivate_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    receipt,
                )?;
            }
            IncomingStakingTransactionData::UnparkValidator {
                proof: signature, ..
            } => {
                let validator_address = signature.compute_signer();

                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                StakingContract::revert_unpark_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    receipt,
                )?;
            }
            IncomingStakingTransactionData::CreateStaker {
                proof: signature, ..
            } => {
                let staker_address = signature.compute_signer();

                StakingContract::revert_create_staker(accounts_tree, db_txn, &staker_address)?;
            }
            IncomingStakingTransactionData::Stake { staker_address } => {
                StakingContract::revert_stake(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    transaction.value,
                )?;
            }

            IncomingStakingTransactionData::UpdateStaker {
                proof: signature, ..
            } => {
                let staker_address = signature.compute_signer();

                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                StakingContract::revert_update_staker(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    receipt,
                )?;
            }
            IncomingStakingTransactionData::RetireStaker {
                value,
                proof: signature,
            } => {
                let staker_address = signature.compute_signer();

                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                StakingContract::revert_retire_staker(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    value,
                    receipt,
                )?;
            }
            IncomingStakingTransactionData::ReactivateStaker {
                value,
                proof: signature,
            } => {
                let staker_address = signature.compute_signer();

                StakingContract::revert_reactivate_staker(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    value,
                )?;
            }
        }

        Ok(())
    }

    fn commit_outgoing_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        let receipt;

        let data: OutgoingStakingTransactionProof =
            Deserialize::deserialize(&mut &transaction.proof[..])?;

        match data {
            OutgoingStakingTransactionProof::DropValidator { proof: signature } => {
                let validator_address = signature.compute_signer();

                receipt = Some(
                    StakingContract::drop_validator(
                        accounts_tree,
                        db_txn,
                        &validator_address,
                        block_height,
                    )?
                    .serialize_to_vec(),
                );
            }
            OutgoingStakingTransactionProof::Unstake { proof: signature } => {
                let staker_address = signature.compute_signer();

                receipt = StakingContract::unstake(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    transaction.total_value()?,
                    block_height,
                )?
                .map(|r| r.serialize_to_vec());
            }
            OutgoingStakingTransactionProof::DeductFees {
                from_active_balance,
                proof: signature,
            } => {
                let staker_address = signature.compute_signer();

                receipt = StakingContract::deduct_fees(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    from_active_balance,
                    transaction.fee,
                )?
                .map(|r| r.serialize_to_vec());
            }
        }

        Ok(receipt)
    }

    fn revert_outgoing_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        let data: OutgoingStakingTransactionProof =
            Deserialize::deserialize(&mut &transaction.proof[..])?;

        match data {
            OutgoingStakingTransactionProof::DropValidator { proof: signature } => {
                let validator_address = signature.compute_signer();

                let receipt: DropValidatorReceipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                StakingContract::revert_drop_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    receipt,
                )?;
            }
            OutgoingStakingTransactionProof::Unstake { proof: signature } => {
                let staker_address = signature.compute_signer();

                let receipt = match receipt {
                    Some(v) => Some(Deserialize::deserialize_from_vec(v)?),
                    None => None,
                };

                StakingContract::revert_unstake(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    transaction.total_value()?,
                    receipt,
                )?;
            }
            OutgoingStakingTransactionProof::DeductFees {
                from_active_balance,
                proof: signature,
            } => {
                let staker_address = signature.compute_signer();

                let receipt = match receipt {
                    Some(v) => Some(Deserialize::deserialize_from_vec(v)?),
                    None => None,
                };

                StakingContract::revert_deduct_fees(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    from_active_balance,
                    transaction.fee,
                    receipt,
                )?;
            }
        }

        Ok(())
    }
}

impl AccountInherentInteraction for StakingContract {
    fn commit_inherent(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        inherent: &Inherent,
        block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        trace!("check inherent: {:?}", inherent);

        // Inherent slashes nothing.
        if inherent.value != Coin::ZERO {
            return Err(AccountError::InvalidInherent);
        }

        // Get the staking contract main.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let receipt;

        match &inherent.ty {
            InherentType::Slash => {
                // Check data length.
                if inherent.data.len() != SlashedSlot::SIZE {
                    return Err(AccountError::InvalidInherent);
                }

                // Deserialize slot.
                let slot: SlashedSlot = Deserialize::deserialize(&mut &inherent.data[..])?;

                // Check that the slashed validator does exist.
                if StakingContract::get_validator(accounts_tree, db_txn, &slot.validator_address)
                    .is_none()
                {
                    return Err(AccountError::InvalidInherent);
                }

                // Simply add validator address to parking.
                // TODO: The inherent might have originated from a fork proof for the previous epoch.
                //  Right now, we don't care and start the parking period in the epoch the proof has been submitted.
                let newly_parked = staking_contract
                    .current_epoch_parking
                    .insert(slot.validator_address.clone());

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

                if policy::epoch_at(slot.event_block) < policy::epoch_at(block_height) {
                    newly_lost_rewards = !staking_contract
                        .previous_lost_rewards
                        .contains(slot.slot as usize);

                    newly_disabled = false;

                    staking_contract
                        .previous_lost_rewards
                        .insert(slot.slot as usize);
                } else if policy::batch_at(slot.event_block) < policy::batch_at(block_height) {
                    newly_lost_rewards = !staking_contract
                        .previous_lost_rewards
                        .contains(slot.slot as usize);

                    staking_contract
                        .previous_lost_rewards
                        .insert(slot.slot as usize);

                    newly_disabled = staking_contract
                        .current_disabled_slots
                        .entry(slot.validator_address.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(slot.slot);
                } else {
                    newly_lost_rewards = !staking_contract
                        .current_lost_rewards
                        .contains(slot.slot as usize);

                    staking_contract
                        .current_lost_rewards
                        .insert(slot.slot as usize);

                    newly_disabled = staking_contract
                        .current_disabled_slots
                        .entry(slot.validator_address.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(slot.slot);
                }

                // All checks passed, not allowed to fail from here on!
                trace!("Trying to put staking contract in the accounts tree.");

                receipt = Some(
                    SlashReceipt {
                        newly_parked,
                        newly_disabled,
                        newly_lost_rewards,
                    }
                    .serialize_to_vec(),
                );
            }
            InherentType::FinalizeBatch | InherentType::FinalizeEpoch => {
                // Invalid data length
                if !inherent.data.is_empty() {
                    return Err(AccountError::InvalidInherent);
                }

                // Clear the lost rewards set.
                staking_contract.previous_lost_rewards = staking_contract.current_lost_rewards;
                staking_contract.current_lost_rewards = BitSet::new();

                // Parking sets and disabled slots are only cleared on epoch changes.
                if inherent.ty == InherentType::FinalizeEpoch {
                    // But first, retire all validators that have been parked for more than one epoch.
                    for validator_address in staking_contract.previous_epoch_parking {
                        StakingContract::retire_validator(
                            accounts_tree,
                            db_txn,
                            &validator_address,
                            block_height,
                        )?;
                    }

                    // Now we clear the parking set.
                    staking_contract.previous_epoch_parking =
                        staking_contract.current_epoch_parking;
                    staking_contract.current_epoch_parking = BTreeSet::new();

                    // And the disabled slots.
                    // Optimization: We actually only need the old slots for the first batch of the epoch.
                    staking_contract.previous_disabled_slots =
                        staking_contract.current_disabled_slots;
                    staking_contract.current_disabled_slots = BTreeMap::new();
                }

                // Since finalized epochs cannot be reverted, we don't need any receipts.
                receipt = None;
            }
            InherentType::Reward => {
                return Err(AccountError::InvalidForTarget);
            }
        }

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(receipt)
    }

    fn revert_inherent(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        inherent: &Inherent,
        block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        // Get the staking contract main.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        match &inherent.ty {
            InherentType::Slash => {
                let receipt: SlashReceipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                let slot: SlashedSlot = Deserialize::deserialize(&mut &inherent.data[..])?;

                // Only remove if it was not already slashed.
                // I kept this in two nested if's for clarity.
                if receipt.newly_parked {
                    let has_been_removed = staking_contract
                        .current_epoch_parking
                        .remove(&slot.validator_address);
                    if !has_been_removed {
                        return Err(AccountError::InvalidInherent);
                    }
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
                    if policy::epoch_at(slot.event_block) < policy::epoch_at(block_height) {
                        // Nothing to do.
                    } else {
                        let is_empty = {
                            let entry = staking_contract
                                .current_disabled_slots
                                .get_mut(&slot.validator_address)
                                .unwrap();
                            entry.remove(&slot.slot);
                            entry.is_empty()
                        };
                        if is_empty {
                            staking_contract
                                .current_disabled_slots
                                .remove(&slot.validator_address);
                        }
                    }
                }
                if receipt.newly_lost_rewards {
                    if policy::epoch_at(slot.event_block) < policy::epoch_at(block_height)
                        || policy::batch_at(slot.event_block) < policy::batch_at(block_height)
                    {
                        staking_contract
                            .previous_lost_rewards
                            .remove(slot.slot as usize);
                    } else {
                        staking_contract
                            .current_lost_rewards
                            .remove(slot.slot as usize);
                    }
                }
            }
            InherentType::FinalizeBatch | InherentType::FinalizeEpoch => {
                // We should not be able to revert finalized epochs or batches!
                return Err(AccountError::InvalidForTarget);
            }
            InherentType::Reward => {
                return Err(AccountError::InvalidForTarget);
            }
        }

        accounts_tree.put(
            db_txn,
            &StakingContract::get_key_staking_contract(),
            Account::Staking(staking_contract),
        );

        Ok(())
    }
}
