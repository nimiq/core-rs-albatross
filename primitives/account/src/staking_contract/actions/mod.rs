use std::collections::BTreeSet;
use std::mem;
use std::ops::Add;

use beserial::{Deserialize, Serialize};
use nimiq_collections::BitSet;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_primitives::account::{AccountType, ValidatorId};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::policy;
use nimiq_primitives::slots::SlashedSlot;
use nimiq_transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof, SelfStakingTransactionData,
};
use nimiq_transaction::Transaction;

use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::staking_contract::actions::staker::InactiveStakeReceipt;
use crate::staking_contract::actions::validator::{
    DropValidatorReceipt, InactiveValidatorReceipt, UnparkReceipt, UpdateValidatorReceipt,
};
use crate::staking_contract::SlashReceipt;
use crate::{Account, AccountError, Inherent, InherentType, StakingContract};

pub mod staker;
pub mod validator;

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
    fn new_contract(
        _: AccountType,
        _: Coin,
        _: &Transaction,
        _: u32,
        _: u64,
    ) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn create(_: Coin, _: &Transaction, _: u32, _: u64) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn check_incoming_transaction(_: &Transaction, _: u32, _: u64) -> Result<(), AccountError> {
        Ok(())
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        _time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        if transaction.sender != transaction.recipient {
            // Stake transaction
            let data = IncomingStakingTransactionData::parse(transaction)?;

            let receipt: Option<Vec<u8>> = match data {
                IncomingStakingTransactionData::CreateValidator {
                    validator_key,
                    reward_address,
                    ..
                } => {
                    // Create validator id from creation tx hash
                    let validator_id: ValidatorId =
                        transaction.hash::<Blake2bHash>().as_slice()[0..20].into();
                    self.create_validator(
                        validator_id,
                        validator_key,
                        reward_address,
                        transaction.value,
                    )?;
                    None
                }
                IncomingStakingTransactionData::UpdateValidator {
                    validator_id,
                    old_validator_key: _,
                    new_validator_key,
                    new_reward_address,
                    signature,
                    ..
                } => {
                    // We couldn't verify the signature intrinsically before, since we need the validator key from the staking contract
                    self.verify_signature_incoming(transaction, &validator_id, &signature)?;

                    let receipt = self.update_validator(
                        &validator_id,
                        new_validator_key,
                        new_reward_address,
                    )?;
                    Some(receipt.serialize_to_vec())
                }
                IncomingStakingTransactionData::RetireValidator {
                    validator_id,
                    signature,
                } => {
                    self.verify_signature_incoming(transaction, &validator_id, &signature)?;
                    self.retire_validator(validator_id, block_height)?;
                    None
                }
                IncomingStakingTransactionData::ReactivateValidator {
                    validator_id,
                    signature,
                } => {
                    self.verify_signature_incoming(transaction, &validator_id, &signature)?;
                    let receipt = self.reactivate_validator(validator_id)?;
                    Some(receipt.serialize_to_vec())
                }
                IncomingStakingTransactionData::UnparkValidator {
                    validator_id,
                    signature,
                } => {
                    self.verify_signature_incoming(transaction, &validator_id, &signature)?;
                    let receipt = self.unpark_validator(&validator_id)?;
                    Some(receipt.serialize_to_vec())
                }
                IncomingStakingTransactionData::Stake {
                    validator_id,
                    staker_address,
                } => {
                    let staker_address =
                        staker_address.unwrap_or_else(|| transaction.sender.clone());
                    self.stake(staker_address, transaction.value, &validator_id)?;
                    None
                }
            };
            Ok(receipt)
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            // XXX Get staker address from transaction proof. This violates the model that only the
            // sender account should evaluate the proof. However, retire/unpark are self transactions, so
            // this contract is both sender and receiver.
            let staker_address = Self::get_self_signer(transaction)?;

            match data {
                SelfStakingTransactionData::RetireStake(_) => {
                    // Retire transaction.
                    Ok(self
                        .retire_recipient(&staker_address, transaction.value, Some(block_height))?
                        .map(|receipt| receipt.serialize_to_vec()))
                }
                SelfStakingTransactionData::ReactivateStake(validator_id) => {
                    self.reactivate_recipient(staker_address, transaction.value, &validator_id)?;
                    Ok(None)
                }
                SelfStakingTransactionData::RededicateStake {
                    from_validator_id: _,
                    to_validator_id,
                } => {
                    self.rededicate_stake_receiver(
                        staker_address,
                        transaction.value,
                        &to_validator_id,
                    )?;
                    Ok(None)
                }
            }
        }
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_height: u32,
        _time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if transaction.sender != transaction.recipient {
            let data: IncomingStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;

            match data {
                IncomingStakingTransactionData::CreateValidator { validator_key, .. } => {
                    // Validator id was generated from creation tx hash
                    let validator_id: ValidatorId =
                        transaction.hash::<Blake2bHash>().as_slice()[0..20].into();
                    self.revert_create_validator(validator_id, validator_key, transaction.value)?;
                }
                IncomingStakingTransactionData::UpdateValidator {
                    validator_id,
                    old_validator_key,
                    new_validator_key: _,
                    ..
                } => {
                    let receipt: UpdateValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    self.revert_update_validator(validator_id, old_validator_key, receipt)?;
                }
                IncomingStakingTransactionData::RetireValidator { validator_id, .. } => {
                    self.revert_retire_validator(validator_id)?;
                }
                IncomingStakingTransactionData::ReactivateValidator { validator_id, .. } => {
                    let receipt: InactiveValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    self.revert_reactivate_validator(validator_id, receipt)?;
                }
                IncomingStakingTransactionData::UnparkValidator { validator_id, .. } => {
                    let receipt: UnparkReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    self.revert_unpark_validator(&validator_id, receipt)?;
                }
                IncomingStakingTransactionData::Stake {
                    validator_id,
                    staker_address,
                } => {
                    let staker_address_ref = staker_address.as_ref().unwrap_or(&transaction.sender);
                    self.revert_stake(staker_address_ref, transaction.value, &validator_id)?;
                }
            }
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            let staker_address = Self::get_self_signer(transaction)?;

            match data {
                SelfStakingTransactionData::RetireStake(_) => {
                    let receipt: Option<InactiveStakeReceipt> = conditional_deserialize(receipt)?;
                    // Retire transaction.
                    self.revert_retire_recipient(&staker_address, transaction.value, receipt)?;
                }
                SelfStakingTransactionData::ReactivateStake(validator_key) => {
                    self.revert_reactivate_recipient(
                        &staker_address,
                        transaction.value,
                        &validator_key,
                    )?;
                }
                SelfStakingTransactionData::RededicateStake {
                    from_validator_id: _,
                    to_validator_id,
                } => {
                    self.revert_rededicate_stake_receiver(
                        staker_address,
                        transaction.value,
                        &to_validator_id,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn check_outgoing_transaction(
        &self,
        transaction: &Transaction,
        block_height: u32,
        _time: u64,
    ) -> Result<(), AccountError> {
        if transaction.sender != transaction.recipient {
            let proof: OutgoingStakingTransactionProof =
                Deserialize::deserialize(&mut &transaction.proof[..])?;

            match proof {
                OutgoingStakingTransactionProof::Unstake(proof) => {
                    let staker_address = proof.compute_signer();
                    // Unstake transaction.
                    let inactive_stake = self
                        .inactive_stake_by_address
                        .get(&staker_address)
                        .ok_or(AccountError::InvalidForSender)?;

                    // Check unstaking delay.
                    if block_height < policy::election_block_after(inactive_stake.retire_time) {
                        return Err(AccountError::InvalidForSender);
                    }

                    Account::balance_sufficient(inactive_stake.balance, transaction.total_value()?)
                }
                OutgoingStakingTransactionProof::DropValidator { validator_id, .. } => {
                    let inactive_validator = self
                        .inactive_validators_by_id
                        .get(&validator_id)
                        .ok_or(AccountError::InvalidForSender)?;

                    // Check unstaking delay.
                    if block_height < policy::election_block_after(inactive_validator.retire_time) {
                        return Err(AccountError::InvalidForSender);
                    }

                    // Check stakes.
                    let validator = &inactive_validator.validator;
                    let staker_stake = validator
                        .active_stake_by_address
                        .read()
                        .values()
                        .cloned()
                        .fold(Coin::ZERO, Add::add);
                    if validator.balance - staker_stake != transaction.total_value()? {
                        Err(AccountError::InsufficientFunds {
                            needed: staker_stake + transaction.total_value()?,
                            balance: validator.balance,
                        })
                    } else {
                        Ok(())
                    }
                }
            }
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            let staker_address = Self::get_self_signer(transaction)?;

            match data {
                SelfStakingTransactionData::RetireStake(validator_id) => {
                    // Check that there is enough stake for this transaction.
                    let validator = self
                        .get_validator(&validator_id)
                        .ok_or(AccountError::InvalidForSender)?;
                    let stakes = validator.active_stake_by_address.read();
                    let stake = stakes
                        .get(&staker_address)
                        .ok_or(AccountError::InvalidForSender)?;

                    Account::balance_sufficient(*stake, transaction.total_value()?)
                }
                SelfStakingTransactionData::ReactivateStake(validator_id) => {
                    let inactive_stake = self
                        .inactive_stake_by_address
                        .get(&staker_address)
                        .ok_or(AccountError::InvalidForSender)?;

                    // Ensure validator exists.
                    let _ = self
                        .get_validator(&validator_id)
                        .ok_or(AccountError::InvalidForSender)?;

                    Account::balance_sufficient(inactive_stake.balance, transaction.total_value()?)
                }
                SelfStakingTransactionData::RededicateStake {
                    from_validator_id,
                    to_validator_id: _,
                } => {
                    let validator = self
                        .get_validator(&from_validator_id)
                        .ok_or(AccountError::InvalidForSender)?;
                    let stakes = validator.active_stake_by_address.read();
                    let stake = stakes
                        .get(&staker_address)
                        .ok_or(AccountError::InvalidForSender)?;

                    Account::balance_sufficient(*stake, transaction.total_value()?)
                }
            }
        }
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        self.check_outgoing_transaction(transaction, block_height, time)?;

        if transaction.sender != transaction.recipient {
            let proof: OutgoingStakingTransactionProof =
                Deserialize::deserialize(&mut &transaction.proof[..])?;

            Ok(match proof {
                OutgoingStakingTransactionProof::Unstake(proof) => {
                    let staker_address = proof.compute_signer();
                    self.unstake(&staker_address, transaction.total_value()?)?
                        .map(|r| r.serialize_to_vec())
                }
                OutgoingStakingTransactionProof::DropValidator { validator_id, .. } => Some(
                    self.drop_validator(&validator_id, transaction.total_value()?)?
                        .serialize_to_vec(),
                ),
            })
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            let staker_address = Self::get_self_signer(transaction)?;

            Ok(match data {
                SelfStakingTransactionData::RetireStake(validator_id) => {
                    self.retire_sender(&staker_address, transaction.total_value()?, &validator_id)?;
                    None
                }
                SelfStakingTransactionData::ReactivateStake(_validator_id) => self
                    .reactivate_sender(&staker_address, transaction.total_value()?, None)?
                    .map(|r| r.serialize_to_vec()),
                SelfStakingTransactionData::RededicateStake {
                    from_validator_id,
                    to_validator_id: _,
                } => {
                    self.rededicate_stake_sender(
                        staker_address,
                        transaction.total_value()?,
                        &from_validator_id,
                    )?;
                    None
                }
            })
        }
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_height: u32,
        _time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if transaction.sender != transaction.recipient {
            let proof: OutgoingStakingTransactionProof =
                Deserialize::deserialize(&mut &transaction.proof[..])?;

            match proof {
                OutgoingStakingTransactionProof::Unstake(proof) => {
                    let staker_address = proof.compute_signer();
                    let receipt: Option<InactiveStakeReceipt> = conditional_deserialize(receipt)?;
                    self.revert_unstake(&staker_address, transaction.total_value()?, receipt)?;
                }
                OutgoingStakingTransactionProof::DropValidator {
                    validator_id,
                    validator_key,
                    ..
                } => {
                    let receipt: DropValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    self.revert_drop_validator(
                        validator_id,
                        validator_key,
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
                SelfStakingTransactionData::RetireStake(validator_id) => {
                    self.revert_retire_sender(
                        staker_address,
                        transaction.total_value()?,
                        &validator_id,
                    )?;
                }
                SelfStakingTransactionData::ReactivateStake(_validator_id) => {
                    let receipt: Option<InactiveStakeReceipt> = conditional_deserialize(receipt)?;
                    self.revert_reactivate_sender(
                        &staker_address,
                        transaction.total_value()?,
                        receipt,
                    )?;
                }
                SelfStakingTransactionData::RededicateStake {
                    from_validator_id,
                    to_validator_id: _,
                } => {
                    self.revert_rededicate_stake_sender(
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
    fn check_inherent(
        &self,
        inherent: &Inherent,
        _block_height: u32,
        _time: u64,
    ) -> Result<(), AccountError> {
        trace!("check inherent: {:?}", inherent);
        // Inherent slashes nothing
        if inherent.value != Coin::ZERO {
            return Err(AccountError::InvalidInherent);
        }

        match inherent.ty {
            InherentType::Slash => {
                // Invalid data length
                // FIXME: Check that data length matches the correct SlashedSlot struct size.
                // Since SlashedSlot uses a ValidatorId, it has to be at least bigger than that.
                if inherent.data.len() < ValidatorId::SIZE {
                    return Err(AccountError::InvalidInherent);
                }

                // Address doesn't exist in contract
                let slot: SlashedSlot = Deserialize::deserialize(&mut &inherent.data[..])?;
                if !self
                    .active_validators_by_id
                    .contains_key(&slot.validator_id)
                    && !self
                        .inactive_validators_by_id
                        .contains_key(&slot.validator_id)
                {
                    return Err(AccountError::InvalidInherent);
                }

                Ok(())
            }
            InherentType::FinalizeBatch | InherentType::FinalizeEpoch => {
                // Invalid data length
                if !inherent.data.is_empty() {
                    return Err(AccountError::InvalidInherent);
                }

                Ok(())
            }
            InherentType::Reward => Err(AccountError::InvalidForTarget),
        }
    }

    fn commit_inherent(
        &mut self,
        inherent: &Inherent,
        block_height: u32,
        time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        self.check_inherent(inherent, block_height, time)?;

        match &inherent.ty {
            InherentType::Slash => {
                // Simply add validator address to parking.
                let slot: SlashedSlot = Deserialize::deserialize(&mut &inherent.data[..])?;
                // TODO: The inherent might have originated from a fork proof for the previous epoch.
                // Right now, we don't care and start the parking period in the epoch the proof has been submitted.
                let newly_parked = self.current_epoch_parking.insert(slot.validator_id.clone());

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
                    newly_lost_rewards = !self.previous_lost_rewards.contains(slot.slot as usize);
                    newly_disabled = false;
                    self.previous_lost_rewards.insert(slot.slot as usize);
                } else if policy::batch_at(slot.event_block) < policy::batch_at(block_height) {
                    newly_lost_rewards = !self.previous_lost_rewards.contains(slot.slot as usize);
                    self.previous_lost_rewards.insert(slot.slot as usize);
                    newly_disabled = self
                        .current_disabled_slots
                        .entry(slot.validator_id.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(slot.slot);
                } else {
                    newly_lost_rewards = !self.current_lost_rewards.contains(slot.slot as usize);
                    self.current_lost_rewards.insert(slot.slot as usize);
                    newly_disabled = self
                        .current_disabled_slots
                        .entry(slot.validator_id.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(slot.slot);
                }

                let receipt = SlashReceipt {
                    newly_parked,
                    newly_disabled,
                    newly_lost_rewards,
                };
                Ok(Some(receipt.serialize_to_vec()))
            }
            InherentType::FinalizeBatch | InherentType::FinalizeEpoch => {
                // Lost rewards.
                let current_lost_rewards =
                    mem::replace(&mut self.current_lost_rewards, BitSet::new());
                let _old_lost_rewards =
                    mem::replace(&mut self.previous_lost_rewards, current_lost_rewards);

                // Parking sets and disabled slots are only swapped on epoch changes.
                if inherent.ty == InherentType::FinalizeEpoch {
                    // Swap lists around.
                    let current_epoch = std::mem::take(&mut self.current_epoch_parking);
                    let old_epoch = mem::replace(&mut self.previous_epoch_parking, current_epoch);

                    // Disabled slots.
                    // Optimization: We actually only need the old slots for the first batch of the epoch.
                    let current_disabled_slots = std::mem::take(&mut self.current_disabled_slots);
                    let _old_disabled_slots =
                        mem::replace(&mut self.previous_disabled_slots, current_disabled_slots);

                    // Remove all parked validators.
                    for validator_id in old_epoch {
                        // We do not remove validators from the parking list if they send a retire transaction.
                        // Instead, we simply skip these here.
                        // This saves space in the receipts of retire transactions as they happen much more often
                        // than stakers are added to the parking lists.
                        if self.active_validators_by_id.contains_key(&validator_id) {
                            self.retire_validator(validator_id, block_height)?;
                        }
                    }
                }

                // Since finalized epochs cannot be reverted, we don't need any receipts.
                Ok(None)
            }
            _ => unreachable!(),
        }
    }

    fn revert_inherent(
        &mut self,
        inherent: &Inherent,
        block_height: u32,
        _time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        match &inherent.ty {
            InherentType::Slash => {
                let receipt: SlashReceipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;
                let slot: SlashedSlot = Deserialize::deserialize(&mut &inherent.data[..])?;

                // Only remove if it was not already slashed.
                // I kept this in two nested if's for clarity.
                if receipt.newly_parked {
                    let has_been_removed = self.current_epoch_parking.remove(&slot.validator_id);
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
                            let entry = self
                                .current_disabled_slots
                                .get_mut(&slot.validator_id)
                                .unwrap();
                            entry.remove(&slot.slot);
                            entry.is_empty()
                        };
                        if is_empty {
                            self.current_disabled_slots.remove(&slot.validator_id);
                        }
                    }
                }

                if receipt.newly_lost_rewards {
                    if policy::epoch_at(slot.event_block) < policy::epoch_at(block_height)
                        || policy::batch_at(slot.event_block) < policy::batch_at(block_height)
                    {
                        self.previous_lost_rewards.remove(slot.slot as usize);
                    } else {
                        self.current_lost_rewards.remove(slot.slot as usize);
                    }
                }
            }
            InherentType::FinalizeBatch => {
                // We should not be able to revert finalized epochs!
                return Err(AccountError::InvalidForTarget);
            }
            _ => unreachable!(),
        }

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
