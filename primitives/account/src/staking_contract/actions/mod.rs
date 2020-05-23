use std::collections::HashSet;
use std::mem;
use std::ops::Add;

use beserial::{Deserialize, Serialize};
use bls::CompressedPublicKey as BlsPublicKey;
use primitives::coin::Coin;
use primitives::policy;
use transaction::account::staking_contract::{
    IncomingStakingTransactionData, OutgoingStakingTransactionProof, SelfStakingTransactionData,
};
use transaction::Transaction;

use crate::inherent::AccountInherentInteraction;
use crate::staking_contract::actions::staker::InactiveStakeReceipt;
use crate::staking_contract::actions::validator::{
    DropValidatorReceipt, InactiveValidatorReceipt, UnparkReceipt, UpdateValidatorReceipt,
};
use crate::staking_contract::SlashReceipt;
use crate::{
    Account, AccountError, AccountTransactionInteraction, AccountType, Inherent, InherentType,
    StakingContract,
};

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
    ) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn create(_: Coin, _: &Transaction, _: u32) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn check_incoming_transaction(_: &Transaction, _: u32) -> Result<(), AccountError> {
        Ok(())
    }

    fn commit_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
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
                    self.create_validator(validator_key, reward_address, transaction.value)?;
                    None
                }
                IncomingStakingTransactionData::UpdateValidator {
                    old_validator_key,
                    new_validator_key,
                    new_reward_address,
                    ..
                } => {
                    let receipt = self.update_validator(
                        &old_validator_key,
                        new_validator_key,
                        new_reward_address,
                    )?;
                    Some(receipt.serialize_to_vec())
                }
                IncomingStakingTransactionData::RetireValidator { validator_key, .. } => {
                    self.retire_validator(validator_key, block_height)?;
                    None
                }
                IncomingStakingTransactionData::ReactivateValidator { validator_key, .. } => {
                    let receipt = self.reactivate_validator(validator_key)?;
                    Some(receipt.serialize_to_vec())
                }
                IncomingStakingTransactionData::UnparkValidator { validator_key, .. } => {
                    let receipt = self.unpark_validator(&validator_key)?;
                    Some(receipt.serialize_to_vec())
                }
                IncomingStakingTransactionData::Stake {
                    validator_key,
                    staker_address,
                } => {
                    let staker_address =
                        staker_address.unwrap_or_else(|| transaction.sender.clone());
                    self.stake(staker_address, transaction.value, &validator_key)?;
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
                SelfStakingTransactionData::ReactivateStake(validator_key) => {
                    self.reactivate_recipient(staker_address, transaction.value, &validator_key)?;
                    Ok(None)
                }
            }
        }
    }

    fn revert_incoming_transaction(
        &mut self,
        transaction: &Transaction,
        _block_height: u32,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if transaction.sender != transaction.recipient {
            let data: IncomingStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;

            match data {
                IncomingStakingTransactionData::CreateValidator { validator_key, .. } => {
                    self.revert_create_validator(validator_key, transaction.value)?;
                }
                IncomingStakingTransactionData::UpdateValidator {
                    old_validator_key,
                    new_validator_key,
                    ..
                } => {
                    let receipt: UpdateValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    self.revert_update_validator(old_validator_key, new_validator_key, receipt)?;
                }
                IncomingStakingTransactionData::RetireValidator { validator_key, .. } => {
                    self.revert_retire_validator(validator_key)?;
                }
                IncomingStakingTransactionData::ReactivateValidator { validator_key, .. } => {
                    let receipt: InactiveValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    self.revert_reactivate_validator(validator_key, receipt)?;
                }
                IncomingStakingTransactionData::UnparkValidator { validator_key, .. } => {
                    let receipt: UnparkReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    self.revert_unpark_validator(&validator_key, receipt)?;
                }
                IncomingStakingTransactionData::Stake {
                    validator_key,
                    staker_address,
                } => {
                    let staker_address_ref = staker_address
                        .as_ref()
                        .unwrap_or_else(|| &transaction.sender);
                    self.revert_stake(staker_address_ref, transaction.value, &validator_key)?;
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
            }
        }
        Ok(())
    }

    fn check_outgoing_transaction(
        &self,
        transaction: &Transaction,
        block_height: u32,
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

                    // Check unstake delay.
                    if block_height
                        < policy::macro_block_after(inactive_stake.retire_time)
                            + policy::UNSTAKING_DELAY
                    {
                        return Err(AccountError::InvalidForSender);
                    }

                    Account::balance_sufficient(inactive_stake.balance, transaction.total_value()?)
                }
                OutgoingStakingTransactionProof::DropValidator { validator_key, .. } => {
                    let inactive_validator = self
                        .inactive_validators_by_key
                        .get(&validator_key)
                        .ok_or(AccountError::InvalidForSender)?;

                    // Check unstake delay.
                    if block_height
                        < policy::macro_block_after(inactive_validator.retire_time)
                            + policy::UNSTAKING_DELAY
                    {
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
                SelfStakingTransactionData::RetireStake(validator_key) => {
                    // Check that there is enough stake for this transaction.
                    let validator = self
                        .get_validator(&validator_key)
                        .ok_or(AccountError::InvalidForSender)?;
                    let stakes = validator.active_stake_by_address.read();
                    let stake = stakes
                        .get(&staker_address)
                        .ok_or(AccountError::InvalidForSender)?;

                    Account::balance_sufficient(*stake, transaction.total_value()?)
                }
                SelfStakingTransactionData::ReactivateStake(validator_key) => {
                    let inactive_stake = self
                        .inactive_stake_by_address
                        .get(&staker_address)
                        .ok_or(AccountError::InvalidForSender)?;

                    // Ensure validator exists.
                    let _ = self
                        .get_validator(&validator_key)
                        .ok_or(AccountError::InvalidForSender)?;

                    Account::balance_sufficient(inactive_stake.balance, transaction.total_value()?)
                }
            }
        }
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        self.check_outgoing_transaction(transaction, block_height)?;

        if transaction.sender != transaction.recipient {
            let proof: OutgoingStakingTransactionProof =
                Deserialize::deserialize(&mut &transaction.proof[..])?;

            Ok(match proof {
                OutgoingStakingTransactionProof::Unstake(proof) => {
                    let staker_address = proof.compute_signer();
                    self.unstake(&staker_address, transaction.total_value()?)?
                        .map(|r| r.serialize_to_vec())
                }
                OutgoingStakingTransactionProof::DropValidator { validator_key, .. } => Some(
                    self.drop_validator(&validator_key, transaction.total_value()?)?
                        .serialize_to_vec(),
                ),
            })
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            let staker_address = Self::get_self_signer(transaction)?;

            Ok(match data {
                SelfStakingTransactionData::RetireStake(validator_key) => {
                    self.retire_sender(
                        &staker_address,
                        transaction.total_value()?,
                        &validator_key,
                    )?;
                    None
                }
                SelfStakingTransactionData::ReactivateStake(_validator_key) => self
                    .reactivate_sender(&staker_address, transaction.total_value()?, None)?
                    .map(|r| r.serialize_to_vec()),
            })
        }
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_height: u32,
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
                OutgoingStakingTransactionProof::DropValidator { validator_key, .. } => {
                    let receipt: DropValidatorReceipt = Deserialize::deserialize_from_vec(
                        receipt.ok_or(AccountError::InvalidReceipt)?,
                    )?;
                    self.revert_drop_validator(validator_key, transaction.total_value()?, receipt)?;
                }
            }
        } else {
            let data: SelfStakingTransactionData =
                Deserialize::deserialize(&mut &transaction.data[..])?;
            let staker_address = Self::get_self_signer(transaction)?;

            match data {
                SelfStakingTransactionData::RetireStake(validator_key) => {
                    self.revert_retire_sender(
                        staker_address,
                        transaction.total_value()?,
                        &validator_key,
                    )?;
                }
                SelfStakingTransactionData::ReactivateStake(_validator_key) => {
                    let receipt: Option<InactiveStakeReceipt> = conditional_deserialize(receipt)?;
                    self.revert_reactivate_sender(
                        &staker_address,
                        transaction.total_value()?,
                        receipt,
                    )?;
                }
            }
        }
        Ok(())
    }
}

impl AccountInherentInteraction for StakingContract {
    fn check_inherent(&self, inherent: &Inherent, _block_height: u32) -> Result<(), AccountError> {
        trace!("check inherent: {:?}", inherent);
        // Inherent slashes nothing
        if inherent.value != Coin::ZERO {
            return Err(AccountError::InvalidInherent);
        }

        match inherent.ty {
            InherentType::Slash => {
                // Invalid data length
                if inherent.data.len() != BlsPublicKey::SIZE {
                    return Err(AccountError::InvalidInherent);
                }

                // Address doesn't exist in contract
                let validator_key: BlsPublicKey =
                    Deserialize::deserialize(&mut &inherent.data[..])?;
                if !self.active_validators_by_key.contains_key(&validator_key)
                    && !self.inactive_validators_by_key.contains_key(&validator_key)
                {
                    return Err(AccountError::InvalidInherent);
                }

                Ok(())
            }
            InherentType::FinalizeEpoch => {
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
    ) -> Result<Option<Vec<u8>>, AccountError> {
        self.check_inherent(inherent, block_height)?;

        match &inherent.ty {
            InherentType::Slash => {
                // Simply add validator address to parking.
                let validator_key: BlsPublicKey =
                    Deserialize::deserialize(&mut &inherent.data[..])?;
                // TODO: The inherent might have originated from a fork proof for the previous epoch.
                // Right now, we don't care and start the parking period in the epoch the proof has been submitted.
                let newly_slashed = self.current_epoch_parking.insert(validator_key);
                let receipt = SlashReceipt { newly_slashed };
                Ok(Some(receipt.serialize_to_vec()))
            }
            InherentType::FinalizeEpoch => {
                // Swap lists around.
                let current_epoch = mem::replace(&mut self.current_epoch_parking, HashSet::new());
                let old_epoch = mem::replace(&mut self.previous_epoch_parking, current_epoch);

                // Remove all parked validators.
                for validator_key in old_epoch {
                    // We do not remove validators from the parking list if they send a retire transaction.
                    // Instead, we simply skip these here.
                    // This saves space in the receipts of retire transactions as they happen much more often
                    // than stakers are added to the parking lists.
                    if self.active_validators_by_key.contains_key(&validator_key) {
                        self.retire_validator(validator_key, block_height)?;
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
        _block_height: u32,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        match &inherent.ty {
            InherentType::Slash => {
                let receipt: SlashReceipt = Deserialize::deserialize_from_vec(
                    &receipt.ok_or(AccountError::InvalidReceipt)?,
                )?;
                let validator_key: BlsPublicKey =
                    Deserialize::deserialize(&mut &inherent.data[..])?;

                // Only remove if it was not already slashed.
                // I kept this in two nested if's for clarity.
                if receipt.newly_slashed {
                    let has_been_removed = self.current_epoch_parking.remove(&validator_key);
                    if !has_been_removed {
                        return Err(AccountError::InvalidInherent);
                    }
                }
            }
            InherentType::FinalizeEpoch => {
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
