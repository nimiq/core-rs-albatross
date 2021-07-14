use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::mem;
use std::ops::Add;

use beserial::{Deserialize, Serialize};
use nimiq_collections::BitSet;
use nimiq_database::WriteTransaction;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::account::{AccountType, ValidatorId};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;
use nimiq_primitives::slots::SlashedSlot;
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof, SelfStakingTransactionData,
};
use nimiq_transaction::Transaction;
use nimiq_trie::key_nibbles::KeyNibbles;

use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::staking_contract::receipts::{
    DropValidatorReceipt, ReactivateValidatorOrStakerReceipt, UnparkValidatorReceipt,
    UpdateValidatorReceipt,
};
use crate::staking_contract::{DropStakerReceipt, RetireValidatorReceipt, SlashReceipt};
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

        if transaction.sender != transaction.recipient {
            // Parse transaction.
            let data = IncomingStakingTransactionData::parse(transaction)?;

            match data {
                IncomingStakingTransactionData::CreateValidator {
                    validator_key,
                    reward_address,
                    signal_data,
                    ..
                } => {
                    // Create validator id from creation tx hash
                    let validator_id: ValidatorId =
                        transaction.hash::<Blake2bHash>().as_slice()[0..20].into();

                    // The proof of knowledge and the value of the transaction were already checked
                    // in the transaction. See the verify function for the
                    // IncomingStakingTransactionData struct.

                    StakingContract::create_validator(
                        accounts_tree,
                        db_txn,
                        &validator_id,
                        reward_address,
                        validator_key,
                        signal_data,
                    )?;
                }
                IncomingStakingTransactionData::UpdateValidator {
                    validator_id,
                    old_validator_key: _,
                    new_reward_address,
                    new_validator_key,
                    new_signal_data,
                    signature,
                    ..
                } => {
                    // We couldn't verify the signature intrinsically before, since we need the
                    // validator key from the staking contract.
                    StakingContract::verify_signature_incoming(
                        accounts_tree,
                        db_txn,
                        &transaction,
                        &validator_id,
                        &signature,
                    )?;

                    receipt = Some(
                        StakingContract::update_validator(
                            accounts_tree,
                            db_txn,
                            &validator_id,
                            new_reward_address,
                            new_validator_key,
                            new_signal_data,
                        )?
                        .serialize_to_vec(),
                    )
                }
                IncomingStakingTransactionData::RetireValidator {
                    validator_id,
                    signature,
                } => {
                    // We couldn't verify the signature intrinsically before, since we need the
                    // validator key from the staking contract.
                    StakingContract::verify_signature_incoming(
                        accounts_tree,
                        db_txn,
                        &transaction,
                        &validator_id,
                        &signature,
                    )?;

                    receipt = Some(
                        StakingContract::retire_validator(
                            accounts_tree,
                            db_txn,
                            &validator_id,
                            block_height,
                        )?
                        .serialize_to_vec(),
                    );
                }
                IncomingStakingTransactionData::ReactivateValidator {
                    validator_id,
                    signature,
                } => {
                    // We couldn't verify the signature intrinsically before, since we need the
                    // validator key from the staking contract.
                    StakingContract::verify_signature_incoming(
                        accounts_tree,
                        db_txn,
                        &transaction,
                        &validator_id,
                        &signature,
                    )?;

                    receipt = Some(
                        StakingContract::reactivate_validator(
                            accounts_tree,
                            db_txn,
                            &validator_id,
                        )?
                        .serialize_to_vec(),
                    );
                }
                IncomingStakingTransactionData::UnparkValidator {
                    validator_id,
                    signature,
                } => {
                    // We couldn't verify the signature intrinsically before, since we need the
                    // validator key from the staking contract.
                    StakingContract::verify_signature_incoming(
                        accounts_tree,
                        db_txn,
                        &transaction,
                        &validator_id,
                        &signature,
                    )?;

                    receipt = Some(
                        StakingContract::unpark_validator(accounts_tree, db_txn, &validator_id)?
                            .serialize_to_vec(),
                    );
                }
                IncomingStakingTransactionData::Stake { staker_address } => {
                    StakingContract::stake(
                        accounts_tree,
                        db_txn,
                        &staker_address,
                        transaction.value,
                    )?;
                }
                IncomingStakingTransactionData::CreateStaker {
                    staker_address,
                    delegation,
                } => {
                    StakingContract::create_staker(
                        accounts_tree,
                        db_txn,
                        &staker_address,
                        transaction.value,
                        delegation,
                    )?;
                }
            }
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            // XXX Get staker address from transaction proof. This violates the model that only the
            // sender account should evaluate the proof. However, retire/unpark are self transactions, so
            // this contract is both sender and receiver.
            let staker_address = StakingContract::get_self_signer(transaction)?;

            match data {
                SelfStakingTransactionData::RetireStaker(_) => {
                    // Retire transaction.
                    receipt = staking
                        .retire_recipient(&staker_address, transaction.value, Some(block_height))?
                        .map(|receipt| receipt.serialize_to_vec());
                }
                SelfStakingTransactionData::ReactivateStaker(validator_id) => {
                    staking.reactivate_recipient(
                        staker_address,
                        transaction.value,
                        &validator_id,
                    )?;
                }
                SelfStakingTransactionData::UpdateStaker {
                    new_delegation: _,
                    to_validator_id,
                } => {
                    staking.rededicate_stake_receiver(
                        staker_address,
                        transaction.value,
                        &to_validator_id,
                    )?;
                }
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
        if transaction.sender != transaction.recipient {
            let data: IncomingStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;

            match data {
                IncomingStakingTransactionData::CreateValidator { .. } => {
                    // Validator id was generated from creation tx hash
                    let validator_id: ValidatorId =
                        transaction.hash::<Blake2bHash>().as_slice()[0..20].into();

                    StakingContract::revert_create_validator(accounts_tree, db_txn, &validator_id)?;
                }
                IncomingStakingTransactionData::UpdateValidator { validator_id, .. } => {
                    let receipt: UpdateValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;

                    StakingContract::revert_update_validator(
                        accounts_tree,
                        db_txn,
                        &validator_id,
                        receipt,
                    )?;
                }
                IncomingStakingTransactionData::RetireValidator { validator_id, .. } => {
                    let receipt: RetireValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;

                    StakingContract::revert_retire_validator(
                        accounts_tree,
                        db_txn,
                        &validator_id,
                        receipt,
                    )?;
                }
                IncomingStakingTransactionData::ReactivateValidator { validator_id, .. } => {
                    let receipt: ReactivateValidatorOrStakerReceipt =
                        Deserialize::deserialize_from_vec(
                            receipt.ok_or(AccountError::InvalidReceipt)?,
                        )?;

                    StakingContract::revert_reactivate_validator(
                        accounts_tree,
                        db_txn,
                        &validator_id,
                        receipt,
                    )?;
                }
                IncomingStakingTransactionData::UnparkValidator { validator_id, .. } => {
                    let receipt: UnparkValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;

                    StakingContract::revert_unpark_validator(
                        accounts_tree,
                        db_txn,
                        &validator_id,
                        receipt,
                    )?;
                }
                IncomingStakingTransactionData::Stake { staker_address } => {
                    StakingContract::revert_stake(
                        accounts_tree,
                        db_txn,
                        &staker_address,
                        transaction.value,
                    )?;
                }
                IncomingStakingTransactionData::CreateStaker { staker_address, .. } => {
                    StakingContract::revert_create_staker(accounts_tree, db_txn, &staker_address)?;
                }
            }
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            let staker_address = StakingContract::get_self_signer(transaction)?;

            match data {
                SelfStakingTransactionData::RetireStaker(_) => {
                    let receipt: Option<InactiveStakeReceipt> = conditional_deserialize(receipt)?;
                    // Retire transaction.
                    staking.revert_retire_recipient(&staker_address, transaction.value, receipt)?;
                }
                SelfStakingTransactionData::ReactivateStaker(validator_key) => {
                    staking.revert_reactivate_recipient(
                        &staker_address,
                        transaction.value,
                        &validator_key,
                    )?;
                }
                SelfStakingTransactionData::UpdateStaker {
                    new_delegation: _,
                    to_validator_id,
                } => {
                    staking.revert_rededicate_stake_receiver(
                        staker_address,
                        transaction.value,
                        &to_validator_id,
                    )?;
                }
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
        let mut receipt = None;

        if transaction.sender != transaction.recipient {
            let proof: OutgoingStakingTransactionProof =
                Deserialize::deserialize(&mut &transaction.proof[..])?;

            match proof {
                OutgoingStakingTransactionProof::DropValidator { validator_id, .. } => {
                    // The signature and the value of the transaction were already checked
                    // in the transaction. See the verify function for the
                    // OutgoingStakingTransactionProof struct.

                    receipt = Some(
                        StakingContract::drop_validator(
                            accounts_tree,
                            db_txn,
                            &validator_id,
                            block_height,
                        )?
                        .serialize_to_vec(),
                    );
                }
                OutgoingStakingTransactionProof::Unstake(proof) => {
                    let staker_address = proof.compute_signer();

                    receipt = StakingContract::unstake(
                        accounts_tree,
                        db_txn,
                        &staker_address,
                        transaction.total_value()?,
                        block_height,
                    )?
                    .map(|r| r.serialize_to_vec());
                }
            }
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            let staker_address = Self::get_self_signer(transaction)?;

            match data {
                SelfStakingTransactionData::RetireStaker(validator_id) => {
                    // Check that there is enough stake for this transaction.
                    let validator = staking
                        .get_validator(&validator_id)
                        .ok_or(AccountError::InvalidForSender)?;

                    let stakes = validator.active_stake_by_address.read();

                    let stake = stakes
                        .get(&staker_address)
                        .ok_or(AccountError::InvalidForSender)?;

                    Account::balance_sub(*stake, transaction.total_value()?)?;

                    drop(stakes);

                    staking.retire_sender(
                        &staker_address,
                        transaction.total_value()?,
                        &validator_id,
                    )?;
                }
                SelfStakingTransactionData::ReactivateStaker(validator_id) => {
                    let inactive_stake = staking
                        .inactive_stake_by_address
                        .get(&staker_address)
                        .ok_or(AccountError::InvalidForSender)?;

                    // Ensure validator exists.
                    staking
                        .get_validator(&validator_id)
                        .ok_or(AccountError::InvalidForSender)?;

                    // Check balance.
                    Account::balance_sub(inactive_stake.balance, transaction.total_value()?)?;

                    receipt = staking
                        .reactivate_sender(&staker_address, transaction.total_value()?, None)?
                        .map(|r| r.serialize_to_vec());
                }
                SelfStakingTransactionData::UpdateStaker {
                    new_delegation: from_validator_id,
                    to_validator_id: _,
                } => {
                    let validator = staking
                        .get_validator(&from_validator_id)
                        .ok_or(AccountError::InvalidForSender)?;

                    let stakes = validator.active_stake_by_address.read();

                    let stake = stakes
                        .get(&staker_address)
                        .ok_or(AccountError::InvalidForSender)?;

                    Account::balance_sub(*stake, transaction.total_value()?)?;

                    drop(stakes);

                    staking.rededicate_stake_sender(
                        staker_address,
                        transaction.total_value()?,
                        &from_validator_id,
                    )?;
                }
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
        if transaction.sender != transaction.recipient {
            let proof: OutgoingStakingTransactionProof =
                Deserialize::deserialize(&mut &transaction.proof[..])?;

            match proof {
                OutgoingStakingTransactionProof::DropValidator {
                    validator_id,
                    validator_key,
                    ..
                } => {
                    let receipt: DropValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;

                    StakingContract::revert_drop_validator(
                        accounts_tree,
                        db_txn,
                        &validator_id,
                        receipt,
                    )?;
                }
                OutgoingStakingTransactionProof::Unstake(proof) => {
                    let staker_address = proof.compute_signer();

                    let receipt: Option<DropStakerReceipt> = conditional_deserialize(receipt)?;

                    StakingContract::revert_unstake(
                        accounts_tree,
                        db_txn,
                        &staker_address,
                        transaction.total_value()?,
                        receipt,
                    )?;
                }
            }
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            let staker_address = Self::get_self_signer(transaction)?;

            match data {
                SelfStakingTransactionData::RetireStaker(validator_id) => {
                    staking.revert_retire_sender(
                        staker_address,
                        transaction.total_value()?,
                        &validator_id,
                    )?;
                }
                SelfStakingTransactionData::ReactivateStaker(_validator_id) => {
                    let receipt: Option<InactiveStakeReceipt> = conditional_deserialize(receipt)?;
                    staking.revert_reactivate_sender(
                        &staker_address,
                        transaction.total_value()?,
                        receipt,
                    )?;
                }
                SelfStakingTransactionData::UpdateStaker {
                    new_delegation: from_validator_id,
                    to_validator_id: _,
                } => {
                    staking.revert_rededicate_stake_sender(
                        staker_address,
                        transaction.total_value()?,
                        &from_validator_id,
                    )?;
                }
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

        let mut receipt = None;

        match &inherent.ty {
            InherentType::Slash => {
                // Check data length.
                if inherent.data.len() != SlashedSlot::SIZE {
                    return Err(AccountError::InvalidInherent);
                }

                // Deserialize slot.
                let slot: SlashedSlot = Deserialize::deserialize(&mut &inherent.data[..])?;

                // Check that the slashed validator does exist.
                if StakingContract::get_validator(accounts_tree, db_txn, validator_id).is_none() {
                    return Err(AccountError::InvalidInherent);
                }

                // Simply add validator address to parking.
                // TODO: The inherent might have originated from a fork proof for the previous epoch.
                //  Right now, we don't care and start the parking period in the epoch the proof has been submitted.
                let newly_parked = staking_contract
                    .current_epoch_parking
                    .insert(slot.validator_id.clone());

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
                        .entry(slot.validator_id.clone())
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
                        .entry(slot.validator_id.clone())
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
                    for validator_id in staking_contract.previous_epoch_parking {
                        StakingContract::retire_validator(
                            accounts_tree,
                            db_txn,
                            &validator_id,
                            block_height,
                        )?;
                    }

                    // Now we clear the parking set.
                    staking_contract.previous_epoch_parking =
                        staking_contract.current_epoch_parking;
                    staking_contract.current_epoch_parking = HashSet::new();

                    // And the disabled slots.
                    // Optimization: We actually only need the old slots for the first batch of the epoch.
                    staking_contract.previous_disabled_slots =
                        staking_contract.current_disabled_slots;
                    staking_contract.current_disabled_slots = HashMap::new();
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
                    &receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                let slot: SlashedSlot = Deserialize::deserialize(&mut &inherent.data[..])?;

                // Only remove if it was not already slashed.
                // I kept this in two nested if's for clarity.
                if receipt.newly_parked {
                    let has_been_removed = staking.current_epoch_parking.remove(&slot.validator_id);
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
                            let entry = staking
                                .current_disabled_slots
                                .get_mut(&slot.validator_id)
                                .unwrap();
                            entry.remove(&slot.slot);
                            entry.is_empty()
                        };
                        if is_empty {
                            staking.current_disabled_slots.remove(&slot.validator_id);
                        }
                    }
                }

                if receipt.newly_lost_rewards {
                    if policy::epoch_at(slot.event_block) < policy::epoch_at(block_height)
                        || policy::batch_at(slot.event_block) < policy::batch_at(block_height)
                    {
                        staking.previous_lost_rewards.remove(slot.slot as usize);
                    } else {
                        staking.current_lost_rewards.remove(slot.slot as usize);
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

        accounts_tree.put(db_txn, &key, Account::Staking(staking));

        Ok(())
    }
}

fn conditional_deserialize<T: Deserialize>(
    receipt: Option<&Vec<u8>>,
) -> Result<Option<T>, AccountError> {
    match receipt {
        Some(v) => Ok(Some(Deserialize::deserialize_from_vec(v)?)),
        _ => Ok(None),
    }
}
