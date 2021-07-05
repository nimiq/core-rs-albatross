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
    DropValidatorReceipt, InactiveStakeReceipt, InactiveValidatorReceipt, UnparkReceipt,
    UpdateValidatorReceipt,
};
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
        let key = KeyNibbles::from(&transaction.recipient);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.recipient.clone(),
            })?;

        let mut staking = match account {
            Account::Staking(value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Staking,
                    got: account.account_type(),
                })
            }
        };

        let mut receipt = None;

        if transaction.sender != transaction.recipient {
            // Stake transaction
            let data = IncomingStakingTransactionData::parse(transaction)?;

            match data {
                IncomingStakingTransactionData::CreateValidator {
                    validator_key,
                    reward_address,
                    ..
                } => {
                    // Create validator id from creation tx hash
                    let validator_id: ValidatorId =
                        transaction.hash::<Blake2bHash>().as_slice()[0..20].into();

                    staking.create_validator(
                        validator_id,
                        validator_key,
                        reward_address,
                        transaction.value,
                    )?;
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
                    staking.verify_signature_incoming(&transaction, &validator_id, &signature)?;

                    receipt = Some(
                        staking
                            .update_validator(&validator_id, new_validator_key, new_reward_address)?
                            .serialize_to_vec(),
                    )
                }
                IncomingStakingTransactionData::RetireValidator {
                    validator_id,
                    signature,
                } => {
                    staking.verify_signature_incoming(&transaction, &validator_id, &signature)?;
                    staking.retire_validator(validator_id, block_height)?;
                }
                IncomingStakingTransactionData::ReactivateValidator {
                    validator_id,
                    signature,
                } => {
                    staking.verify_signature_incoming(&transaction, &validator_id, &signature)?;
                    receipt = Some(
                        staking
                            .reactivate_validator(validator_id)?
                            .serialize_to_vec(),
                    );
                }
                IncomingStakingTransactionData::UnparkValidator {
                    validator_id,
                    signature,
                } => {
                    staking.verify_signature_incoming(&transaction, &validator_id, &signature)?;
                    receipt = Some(staking.unpark_validator(&validator_id)?.serialize_to_vec());
                }
                IncomingStakingTransactionData::Stake {
                    validator_id,
                    staker_address,
                } => {
                    let staker_address =
                        staker_address.unwrap_or_else(|| transaction.sender.clone());
                    staking.stake(staker_address, transaction.value, &validator_id)?;
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
                SelfStakingTransactionData::RetireStake(_) => {
                    // Retire transaction.
                    receipt = staking
                        .retire_recipient(&staker_address, transaction.value, Some(block_height))?
                        .map(|receipt| receipt.serialize_to_vec());
                }
                SelfStakingTransactionData::ReactivateStake(validator_id) => {
                    staking.reactivate_recipient(
                        staker_address,
                        transaction.value,
                        &validator_id,
                    )?;
                }
                SelfStakingTransactionData::RededicateStake {
                    from_validator_id: _,
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

        accounts_tree.put(db_txn, &key, Account::Staking(staking));

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
        let key = KeyNibbles::from(&transaction.recipient);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.recipient.clone(),
            })?;

        let mut staking = match account {
            Account::Staking(value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Staking,
                    got: account.account_type(),
                })
            }
        };

        if transaction.sender != transaction.recipient {
            let data: IncomingStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;

            match data {
                IncomingStakingTransactionData::CreateValidator { validator_key, .. } => {
                    // Validator id was generated from creation tx hash
                    let validator_id: ValidatorId =
                        transaction.hash::<Blake2bHash>().as_slice()[0..20].into();
                    staking.revert_create_validator(
                        validator_id,
                        validator_key,
                        transaction.value,
                    )?;
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
                    staking.revert_update_validator(validator_id, old_validator_key, receipt)?;
                }
                IncomingStakingTransactionData::RetireValidator { validator_id, .. } => {
                    staking.revert_retire_validator(validator_id)?;
                }
                IncomingStakingTransactionData::ReactivateValidator { validator_id, .. } => {
                    let receipt: InactiveValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    staking.revert_reactivate_validator(validator_id, receipt)?;
                }
                IncomingStakingTransactionData::UnparkValidator { validator_id, .. } => {
                    let receipt: UnparkReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    staking.revert_unpark_validator(&validator_id, receipt)?;
                }
                IncomingStakingTransactionData::Stake {
                    validator_id,
                    staker_address,
                } => {
                    let staker_address_ref = staker_address.as_ref().unwrap_or(&transaction.sender);
                    staking.revert_stake(staker_address_ref, transaction.value, &validator_id)?;
                }
            }
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            let staker_address = StakingContract::get_self_signer(transaction)?;

            match data {
                SelfStakingTransactionData::RetireStake(_) => {
                    let receipt: Option<InactiveStakeReceipt> = conditional_deserialize(receipt)?;
                    // Retire transaction.
                    staking.revert_retire_recipient(&staker_address, transaction.value, receipt)?;
                }
                SelfStakingTransactionData::ReactivateStake(validator_key) => {
                    staking.revert_reactivate_recipient(
                        &staker_address,
                        transaction.value,
                        &validator_key,
                    )?;
                }
                SelfStakingTransactionData::RededicateStake {
                    from_validator_id: _,
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

        accounts_tree.put(db_txn, &key, Account::Staking(staking));

        Ok(())
    }

    fn commit_outgoing_transaction(
        accounts_tree: &AccountsTree,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_height: u32,
        _block_time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        let key = KeyNibbles::from(&transaction.sender);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.sender.clone(),
            })?;

        let mut staking = match account {
            Account::Staking(value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Staking,
                    got: account.account_type(),
                })
            }
        };

        let mut receipt = None;

        if transaction.sender != transaction.recipient {
            let proof: OutgoingStakingTransactionProof =
                Deserialize::deserialize(&mut &transaction.proof[..])?;

            match proof {
                OutgoingStakingTransactionProof::Unstake(proof) => {
                    let staker_address = proof.compute_signer();

                    // Unstake transaction.
                    let inactive_stake = staking
                        .inactive_stake_by_address
                        .get(&staker_address)
                        .ok_or(AccountError::InvalidForSender)?;

                    // Check unstaking delay.
                    if block_height < policy::election_block_after(inactive_stake.retire_time) {
                        return Err(AccountError::InvalidForSender);
                    }

                    // Check balance.
                    Account::balance_sub(inactive_stake.balance, transaction.total_value()?)?;

                    receipt = staking
                        .unstake(&staker_address, transaction.total_value()?)?
                        .map(|r| r.serialize_to_vec());
                }
                OutgoingStakingTransactionProof::DropValidator { validator_id, .. } => {
                    let inactive_validator = staking
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
                        return Err(AccountError::InsufficientFunds {
                            needed: staker_stake + transaction.total_value()?,
                            balance: validator.balance,
                        });
                    }

                    receipt = Some(
                        staking
                            .drop_validator(&validator_id, transaction.total_value()?)?
                            .serialize_to_vec(),
                    );
                }
            }
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            let staker_address = Self::get_self_signer(transaction)?;

            match data {
                SelfStakingTransactionData::RetireStake(validator_id) => {
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
                SelfStakingTransactionData::ReactivateStake(validator_id) => {
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
                SelfStakingTransactionData::RededicateStake {
                    from_validator_id,
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

        accounts_tree.put(db_txn, &key, Account::Staking(staking));

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
        let key = KeyNibbles::from(&transaction.sender);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: transaction.sender.clone(),
            })?;

        let mut staking = match account {
            Account::Staking(value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Staking,
                    got: account.account_type(),
                })
            }
        };

        if transaction.sender != transaction.recipient {
            let proof: OutgoingStakingTransactionProof =
                Deserialize::deserialize(&mut &transaction.proof[..])?;

            match proof {
                OutgoingStakingTransactionProof::Unstake(proof) => {
                    let staker_address = proof.compute_signer();
                    let receipt: Option<InactiveStakeReceipt> = conditional_deserialize(receipt)?;
                    staking.revert_unstake(&staker_address, transaction.total_value()?, receipt)?;
                }
                OutgoingStakingTransactionProof::DropValidator {
                    validator_id,
                    validator_key,
                    ..
                } => {
                    let receipt: DropValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    staking.revert_drop_validator(
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
                    staking.revert_retire_sender(
                        staker_address,
                        transaction.total_value()?,
                        &validator_id,
                    )?;
                }
                SelfStakingTransactionData::ReactivateStake(_validator_id) => {
                    let receipt: Option<InactiveStakeReceipt> = conditional_deserialize(receipt)?;
                    staking.revert_reactivate_sender(
                        &staker_address,
                        transaction.total_value()?,
                        receipt,
                    )?;
                }
                SelfStakingTransactionData::RededicateStake {
                    from_validator_id,
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

        accounts_tree.put(db_txn, &key, Account::Staking(staking));

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
        let key = KeyNibbles::from(&inherent.target);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: inherent.target.clone(),
            })?;

        let mut staking = match account {
            Account::Staking(value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Staking,
                    got: account.account_type(),
                })
            }
        };

        trace!("check inherent: {:?}", inherent);
        // Inherent slashes nothing
        if inherent.value != Coin::ZERO {
            return Err(AccountError::InvalidInherent);
        }

        let receipt;

        match &inherent.ty {
            InherentType::Slash => {
                // Invalid data length
                if inherent.data.len() != SlashedSlot::SIZE {
                    return Err(AccountError::InvalidInherent);
                }

                // Address doesn't exist in contract
                let slot: SlashedSlot = Deserialize::deserialize(&mut &inherent.data[..])?;

                if !staking
                    .active_validators_by_id
                    .contains_key(&slot.validator_id)
                    && !staking
                        .inactive_validators_by_id
                        .contains_key(&slot.validator_id)
                {
                    return Err(AccountError::InvalidInherent);
                }

                // Simply add validator address to parking.
                // TODO: The inherent might have originated from a fork proof for the previous epoch.
                // Right now, we don't care and start the parking period in the epoch the proof has been submitted.
                let newly_parked = staking
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
                    newly_lost_rewards =
                        !staking.previous_lost_rewards.contains(slot.slot as usize);
                    newly_disabled = false;
                    staking.previous_lost_rewards.insert(slot.slot as usize);
                } else if policy::batch_at(slot.event_block) < policy::batch_at(block_height) {
                    newly_lost_rewards =
                        !staking.previous_lost_rewards.contains(slot.slot as usize);
                    staking.previous_lost_rewards.insert(slot.slot as usize);
                    newly_disabled = staking
                        .current_disabled_slots
                        .entry(slot.validator_id.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(slot.slot);
                } else {
                    newly_lost_rewards = !staking.current_lost_rewards.contains(slot.slot as usize);
                    staking.current_lost_rewards.insert(slot.slot as usize);
                    newly_disabled = staking
                        .current_disabled_slots
                        .entry(slot.validator_id.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(slot.slot);
                }

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

                // Lost rewards.
                let current_lost_rewards =
                    mem::replace(&mut staking.current_lost_rewards, BitSet::new());
                let _old_lost_rewards =
                    mem::replace(&mut staking.previous_lost_rewards, current_lost_rewards);

                // Parking sets and disabled slots are only swapped on epoch changes.
                if inherent.ty == InherentType::FinalizeEpoch {
                    // Swap lists around.
                    let current_epoch =
                        mem::replace(&mut staking.current_epoch_parking, HashSet::new());
                    let old_epoch =
                        mem::replace(&mut staking.previous_epoch_parking, current_epoch);

                    // Disabled slots.
                    // Optimization: We actually only need the old slots for the first batch of the epoch.
                    let current_disabled_slots =
                        mem::replace(&mut staking.current_disabled_slots, HashMap::new());
                    let _old_disabled_slots =
                        mem::replace(&mut staking.previous_disabled_slots, current_disabled_slots);

                    // Remove all parked validators.
                    for validator_id in old_epoch {
                        // We do not remove validators from the parking list if they send a retire transaction.
                        // Instead, we simply skip these here.
                        // This saves space in the receipts of retire transactions as they happen much more often
                        // than stakers are added to the parking lists.
                        if staking.active_validators_by_id.contains_key(&validator_id) {
                            staking.retire_validator(validator_id, block_height)?;
                        }
                    }
                }

                // Since finalized epochs cannot be reverted, we don't need any receipts.
                receipt = None;
            }
            InherentType::Reward => {
                return Err(AccountError::InvalidForTarget);
            }
        }

        accounts_tree.put(db_txn, &key, Account::Staking(staking));

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
        let key = KeyNibbles::from(&inherent.target);

        let account = accounts_tree
            .get(db_txn, &key)
            .ok_or(AccountError::NonExistentAddress {
                address: inherent.target.clone(),
            })?;

        let mut staking = match account {
            Account::Staking(value) => value,
            _ => {
                return Err(AccountError::TypeMismatch {
                    expected: AccountType::Staking,
                    got: account.account_type(),
                })
            }
        };

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
