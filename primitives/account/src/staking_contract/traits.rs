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
use crate::logs::{AccountInfo, Log};
use crate::staking_contract::receipts::DeleteValidatorReceipt;
use crate::staking_contract::SlashReceipt;
use crate::{Account, AccountError, AccountsTrie, Inherent, InherentType, StakingContract};

/// We need to distinguish between two types of transactions:
/// 1. Incoming transactions, which include:
///     - Validator
///         * Create
///         * Update
///         * Retire
///         * Reactivate
///         * Unpark
///     - Staker
///         * Create
///         * Stake
///         * Update
///         * Retire
///         * Reactivate
///     The type of transaction is given in the data field.
/// 2. Outgoing transactions, which include:
///     - Validator
///         * Drop
///     - Staker
///         * Unstake
///         * Deduct fees
///     The type of transaction is given in the proof field.
impl AccountTransactionInteraction for StakingContract {
    fn create(
        _accounts_tree: &AccountsTrie,
        _db_txn: &mut WriteTransaction,
        _transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
    ) -> Result<AccountInfo, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    /// Commits an incoming transaction to the accounts trie.
    fn commit_incoming_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_height: u32,
        _block_time: u64,
    ) -> Result<AccountInfo, AccountError> {
        // Check that the address is that of the Staking contract.
        if transaction.recipient != policy::STAKING_CONTRACT_ADDRESS {
            return Err(AccountError::InvalidForRecipient);
        }

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

                StakingContract::create_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    signing_key,
                    voting_key,
                    reward_address.clone(),
                    signal_data,
                )?;
                let logs = vec![Log::CreateValidator {
                    validator_address,
                    reward_address,
                }];

                Ok(AccountInfo::new(None, logs))
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

                Ok(StakingContract::update_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    new_signing_key,
                    new_voting_key,
                    new_reward_address,
                    new_signal_data,
                )?
                .into())
            }
            IncomingStakingTransactionData::InactivateValidator {
                validator_address,
                proof,
            } => {
                // Get the signer's address from the proof.
                let signer = proof.compute_signer();

                Ok(StakingContract::inactivate_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    &signer,
                    block_height,
                )?
                .into())
            }
            IncomingStakingTransactionData::ReactivateValidator {
                validator_address,
                proof,
            } => {
                // Get the signer's address from the proof.
                let signer = proof.compute_signer();

                Ok(StakingContract::reactivate_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    &signer,
                )?
                .into())
            }
            IncomingStakingTransactionData::UnparkValidator {
                validator_address,
                proof,
            } => {
                // Get the signer's address from the proof.
                let signer = proof.compute_signer();

                Ok(StakingContract::unpark_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    &signer,
                )?
                .into())
            }
            IncomingStakingTransactionData::CreateStaker { delegation, proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let logs = StakingContract::create_staker(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    transaction.value,
                    delegation,
                )?;

                Ok(AccountInfo::new(None, logs))
            }
            IncomingStakingTransactionData::Stake { staker_address } => {
                let logs = StakingContract::stake(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    transaction.value,
                )?;
                Ok(AccountInfo::new(None, logs))
            }
            IncomingStakingTransactionData::UpdateStaker {
                new_delegation,
                proof,
            } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();
                Ok(StakingContract::update_staker(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    new_delegation,
                )?
                .into())
            }
        }
    }

    /// Reverts the commit of an incoming transaction to the accounts trie.
    fn revert_incoming_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<Vec<Log>, AccountError> {
        // Parse transaction data.
        let data = IncomingStakingTransactionData::parse(transaction)?;

        match data {
            IncomingStakingTransactionData::CreateValidator {
                reward_address,
                proof,
                ..
            } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                StakingContract::revert_create_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                )?;
                Ok(vec![Log::CreateValidator {
                    validator_address,
                    reward_address,
                }])
            }
            IncomingStakingTransactionData::UpdateValidator { proof, .. } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                Ok(StakingContract::revert_update_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    receipt,
                )?)
            }
            IncomingStakingTransactionData::InactivateValidator {
                validator_address, ..
            } => {
                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                Ok(StakingContract::revert_inactivate_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    receipt,
                )?)
            }
            IncomingStakingTransactionData::ReactivateValidator {
                validator_address, ..
            } => {
                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                Ok(StakingContract::revert_reactivate_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    receipt,
                )?)
            }
            IncomingStakingTransactionData::UnparkValidator {
                validator_address, ..
            } => {
                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                Ok(StakingContract::revert_unpark_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    receipt,
                )?)
            }
            IncomingStakingTransactionData::CreateStaker { proof, .. } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                Ok(StakingContract::revert_create_staker(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    transaction.value,
                )?)
            }
            IncomingStakingTransactionData::Stake { staker_address } => {
                Ok(StakingContract::revert_stake(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    transaction.value,
                )?)
            }
            IncomingStakingTransactionData::UpdateStaker { proof, .. } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let receipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                Ok(StakingContract::revert_update_staker(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    receipt,
                )?)
            }
        }
    }

    /// Commits an outgoing transaction to the accounts trie.
    fn commit_outgoing_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        block_height: u32,
        _block_time: u64,
    ) -> Result<AccountInfo, AccountError> {
        // Check that the address is that of the Staking contract.
        if transaction.sender != policy::STAKING_CONTRACT_ADDRESS {
            return Err(AccountError::InvalidForSender);
        }

        // Parse transaction data.
        let data = OutgoingStakingTransactionProof::parse(transaction)?;

        let mut acc_info: AccountInfo;

        match data {
            OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                acc_info = StakingContract::delete_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    block_height,
                )?
                .into();
            }
            OutgoingStakingTransactionProof::Unstake { proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                acc_info = StakingContract::unstake(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    transaction.total_value(),
                )?
                .into();
            }
        };
        acc_info.logs.insert(
            0,
            Log::Transfer {
                from: transaction.sender.clone(),
                to: transaction.recipient.clone(),
                amount: transaction.value,
            },
        );
        acc_info.logs.insert(
            0,
            Log::PayFee {
                from: transaction.sender.clone(),
                fee: transaction.fee,
            },
        );
        Ok(acc_info)
    }

    /// Reverts the commit of an incoming transaction to the accounts trie.
    fn revert_outgoing_transaction(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        transaction: &Transaction,
        _block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<Vec<Log>, AccountError> {
        // Parse transaction data.
        let data = OutgoingStakingTransactionProof::parse(transaction)?;
        let mut logs = vec![
            Log::PayFee {
                from: transaction.sender.clone(),
                fee: transaction.fee,
            },
            Log::Transfer {
                from: transaction.sender.clone(),
                to: transaction.recipient.clone(),
                amount: transaction.value,
            },
        ];
        match data {
            OutgoingStakingTransactionProof::DeleteValidator { proof } => {
                // Get the validator address from the proof.
                let validator_address = proof.compute_signer();

                let receipt: DeleteValidatorReceipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                logs.append(&mut StakingContract::revert_delete_validator(
                    accounts_tree,
                    db_txn,
                    &validator_address,
                    receipt,
                )?);
            }
            OutgoingStakingTransactionProof::Unstake { proof } => {
                // Get the staker address from the proof.
                let staker_address = proof.compute_signer();

                let receipt = match receipt {
                    Some(v) => Some(Deserialize::deserialize_from_vec(v)?),
                    None => None,
                };

                logs.append(&mut StakingContract::revert_unstake(
                    accounts_tree,
                    db_txn,
                    &staker_address,
                    transaction.total_value(),
                    receipt,
                )?);
            }
        }

        Ok(logs)
    }
}

impl AccountInherentInteraction for StakingContract {
    /// Commits an inherent to the accounts trie.
    fn commit_inherent(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        inherent: &Inherent,
        block_height: u32,
        _block_time: u64,
    ) -> Result<AccountInfo, AccountError> {
        trace!("Committing inherent to accounts trie: {:?}", inherent);

        // None of the allowed inherents for the staking contract has a value. Only reward inherents
        // have a value.
        if inherent.value != Coin::ZERO {
            return Err(AccountError::InvalidInherent);
        }

        // Get the staking contract.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);

        let receipt;
        let mut logs = Vec::new();

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

                // Add the validator address to the parked set.
                // TODO: The inherent might have originated from a fork proof for the previous epoch.
                //  Right now, we don't care and start the parking period in the epoch the proof has been submitted.
                let newly_parked = staking_contract
                    .parked_set
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

                    staking_contract
                        .previous_lost_rewards
                        .insert(slot.slot as usize);

                    newly_disabled = false;
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

                receipt = Some(
                    SlashReceipt {
                        newly_parked,
                        newly_disabled,
                        newly_lost_rewards,
                    }
                    .serialize_to_vec(),
                );

                if newly_lost_rewards {
                    logs.push(Log::Slash {
                        validator_address: slot.validator_address.clone(),
                        event_block: slot.event_block,
                        slot: slot.slot,
                        newly_disabled,
                    });
                }
                if newly_parked {
                    logs.push(Log::Park {
                        validator_address: slot.validator_address,
                        event_block: block_height,
                    });
                }
            }
            InherentType::FinalizeBatch | InherentType::FinalizeEpoch => {
                // Invalid data length
                if !inherent.data.is_empty() {
                    return Err(AccountError::InvalidInherent);
                }

                // Clear the lost rewards set.
                staking_contract.previous_lost_rewards = staking_contract.current_lost_rewards;
                staking_contract.current_lost_rewards = BitSet::new();

                // Parking set and disabled slots are only cleared on epoch changes.
                if inherent.ty == InherentType::FinalizeEpoch {
                    // But first, retire all validators that have been parked this epoch.
                    for validator_address in staking_contract.parked_set {
                        // Get the validator and update it.
                        let mut validator = StakingContract::get_validator(
                            accounts_tree,
                            db_txn,
                            &validator_address,
                        )
                        .ok_or(AccountError::InvalidInherent)?;

                        validator.inactivity_flag = Some(block_height);

                        accounts_tree.put(
                            db_txn,
                            &StakingContract::get_key_validator(&validator_address),
                            Account::StakingValidator(validator),
                        );

                        // Update the staking contract.
                        staking_contract
                            .active_validators
                            .remove(&validator_address);

                        logs.push(Log::InactivateValidator {
                            validator_address: validator_address.clone(),
                        });
                    }

                    // Now we clear the parking set.
                    staking_contract.parked_set = BTreeSet::new();

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

        Ok(AccountInfo::new(receipt, logs))
    }

    /// Reverts the commit of an inherent to the accounts trie.
    fn revert_inherent(
        accounts_tree: &AccountsTrie,
        db_txn: &mut WriteTransaction,
        inherent: &Inherent,
        block_height: u32,
        _block_time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<Vec<Log>, AccountError> {
        // Get the staking contract main.
        let mut staking_contract = StakingContract::get_staking_contract(accounts_tree, db_txn);
        let mut logs = Vec::new();

        match &inherent.ty {
            InherentType::Slash => {
                let receipt: SlashReceipt = Deserialize::deserialize_from_vec(
                    receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;

                let slot: SlashedSlot = Deserialize::deserialize(&mut &inherent.data[..])?;

                // Only remove if it was not already slashed.
                if receipt.newly_parked {
                    let has_been_removed =
                        staking_contract.parked_set.remove(&slot.validator_address);

                    if !has_been_removed {
                        return Err(AccountError::InvalidInherent);
                    }
                    logs.push(Log::Park {
                        validator_address: slot.validator_address.clone(),
                        event_block: block_height,
                    });
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
                    logs.insert(
                        0,
                        Log::Slash {
                            validator_address: slot.validator_address,
                            event_block: slot.event_block,
                            slot: slot.slot,
                            newly_disabled: true,
                        },
                    );
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

        Ok(logs)
    }
}
