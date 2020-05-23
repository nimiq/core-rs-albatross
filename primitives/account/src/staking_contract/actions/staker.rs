use beserial::{Deserialize, Serialize};
use bls::CompressedPublicKey as BlsPublicKey;
use keys::Address;
use primitives::coin::Coin;

use crate::staking_contract::InactiveStake;
use crate::{Account, AccountError, StakingContract};

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub(super) struct InactiveStakeReceipt {
    retire_time: u32,
}

/// Actions concerning a staker are:
/// 1. Stake: Delegate stake from an outside address to a validator.
/// 2. Retire: Remove stake from a validator and make it inactive
///            (starting the cooldown period for Unstake).
/// 3. Re-activate: Re-delegate inactive stake to a validator.
/// 4. Unstake: Remove inactive stake from the staking contract
///             (after it has been inactive for the cooldown period).
///
/// The actions can be summarized by the following state diagram:
///        +--------+   retire    +----------+
/// stake  |        +------------>+          | unstake
///+------>+ staked |             | inactive +--------->
///        |        +<------------+          |
///        +--------+ re-activate +----------+
///
/// Stake is a transaction from an arbitrary address to the staking contract.
/// Retire and Re-activate are self transactions on the staking address.
/// Unstake is a transaction from the staking contract to an arbitrary address.
impl StakingContract {
    /// Adds funds to stake of `address` for validator `validator_key`.
    /// XXX This is public to fill the genesis staking contract
    pub fn stake(
        &mut self,
        staker_address: Address,
        value: Coin,
        validator_key: &BlsPublicKey,
    ) -> Result<(), AccountError> {
        let new_balance = Account::balance_add(self.balance, value)?;

        let mut entry = self
            .remove_validator(validator_key)
            .ok_or(AccountError::InvalidForRecipient)?;
        entry.try_add_stake(staker_address, value);
        self.restore_validator(entry)?;

        self.balance = new_balance;

        Ok(())
    }

    /// Reverts a stake transaction.
    pub(super) fn revert_stake(
        &mut self,
        staker_address: &Address,
        value: Coin,
        validator_key: &BlsPublicKey,
    ) -> Result<(), AccountError> {
        let new_balance = Account::balance_sub(self.balance, value)?;

        let mut entry = self
            .remove_validator(validator_key)
            .ok_or(AccountError::InvalidForRecipient)?;
        entry.try_sub_stake(staker_address, value, AccountError::InvalidForRecipient);
        self.restore_validator(entry)?;

        self.balance = new_balance;

        Ok(())
    }

    /// Removes stake from the active stake list.
    pub(super) fn retire_sender(
        &mut self,
        staker_address: &Address,
        value: Coin,
        validator_key: &BlsPublicKey,
    ) -> Result<(), AccountError> {
        let new_balance = Account::balance_sub(self.balance, value)?;

        let mut entry = self
            .remove_validator(validator_key)
            .ok_or(AccountError::InvalidForSender)?;
        entry.try_sub_stake(staker_address, value, AccountError::InvalidForSender);
        self.restore_validator(entry)?;

        self.balance = new_balance;

        Ok(())
    }

    /// Reverts the sender side of a retire transaction.
    pub(super) fn revert_retire_sender(
        &mut self,
        staker_address: Address,
        value: Coin,
        validator_key: &BlsPublicKey,
    ) -> Result<(), AccountError> {
        let new_balance = Account::balance_add(self.balance, value)?;

        let mut entry = self
            .remove_validator(validator_key)
            .ok_or(AccountError::InvalidForSender)?;
        entry.try_add_stake(staker_address, value);
        self.restore_validator(entry)?;

        self.balance = new_balance;

        Ok(())
    }

    /// Adds state to the inactive stake list.
    pub(super) fn retire_recipient(
        &mut self,
        staker_address: &Address,
        value: Coin,
        new_retire_time: Option<u32>,
    ) -> Result<Option<InactiveStakeReceipt>, AccountError> {
        // Make sure new_retire_time is set when entry is not present yet.
        if !self.inactive_stake_by_address.contains_key(staker_address) && new_retire_time.is_none()
        {
            return Err(AccountError::InvalidReceipt);
        }

        self.balance = Account::balance_add(self.balance, value)?;

        // All checks passed, not allowed to fail from here on!
        if let Some(inactive_stake) = self.inactive_stake_by_address.remove(staker_address) {
            let new_inactive_stake = InactiveStake {
                balance: Account::balance_add(inactive_stake.balance, value)?,
                retire_time: new_retire_time.unwrap_or(inactive_stake.retire_time),
            };
            self.inactive_stake_by_address
                .insert(staker_address.clone(), new_inactive_stake);

            Ok(Some(InactiveStakeReceipt {
                retire_time: inactive_stake.retire_time,
            }))
        } else {
            let new_inactive_stake = InactiveStake {
                balance: value,
                retire_time: new_retire_time.unwrap(),
            };
            self.inactive_stake_by_address
                .insert(staker_address.clone(), new_inactive_stake);

            Ok(None)
        }
    }

    /// Reverts a retire transaction.
    pub(super) fn revert_retire_recipient(
        &mut self,
        staker_address: &Address,
        value: Coin,
        receipt: Option<InactiveStakeReceipt>,
    ) -> Result<(), AccountError> {
        let inactive_stake = self
            .inactive_stake_by_address
            .get(staker_address)
            .ok_or(AccountError::InvalidForRecipient)?;

        if (inactive_stake.balance > value) != receipt.is_some() {
            return Err(AccountError::InvalidForRecipient);
        }

        let retire_time = receipt.map(|r| r.retire_time).unwrap_or_default();
        self.reactivate_sender(staker_address, value, Some(retire_time))
            .map(|_| ())
    }

    /// Reactivates stake (sender side).
    pub(super) fn reactivate_sender(
        &mut self,
        staker_address: &Address,
        value: Coin,
        new_retire_time: Option<u32>,
    ) -> Result<Option<InactiveStakeReceipt>, AccountError> {
        let inactive_stake = self
            .inactive_stake_by_address
            .remove(staker_address)
            .ok_or(AccountError::InvalidForRecipient)?;

        self.balance = Account::balance_sub(self.balance, value)?;

        // All checks passed, not allowed to fail from here on!
        if inactive_stake.balance > value {
            let new_inactive_stake = InactiveStake {
                // Balance check is already done in `check` functions.
                balance: Account::balance_sub(inactive_stake.balance, value)?,
                retire_time: new_retire_time.unwrap_or(inactive_stake.retire_time),
            };
            self.inactive_stake_by_address
                .insert(staker_address.clone(), new_inactive_stake);
            Ok(None)
        } else {
            Ok(Some(InactiveStakeReceipt {
                retire_time: inactive_stake.retire_time,
            }))
        }
    }

    /// Adds state to the inactive stake list.
    pub(super) fn revert_reactivate_sender(
        &mut self,
        staker_address: &Address,
        value: Coin,
        receipt: Option<InactiveStakeReceipt>,
    ) -> Result<(), AccountError> {
        self.retire_recipient(staker_address, value, receipt.map(|r| r.retire_time))
            .map(|_| ())
    }

    /// Reactivates stake (recipient side).
    pub(super) fn reactivate_recipient(
        &mut self,
        staker_address: Address,
        value: Coin,
        validator_key: &BlsPublicKey,
    ) -> Result<(), AccountError> {
        self.stake(staker_address, value, validator_key)
    }

    /// Removes stake from the active stake list.
    pub(super) fn revert_reactivate_recipient(
        &mut self,
        staker_address: &Address,
        value: Coin,
        validator_key: &BlsPublicKey,
    ) -> Result<(), AccountError> {
        self.revert_stake(staker_address, value, validator_key)
    }

    /// Removes stake from the inactive stake list.
    pub(super) fn unstake(
        &mut self,
        staker_address: &Address,
        total_value: Coin,
    ) -> Result<Option<InactiveStakeReceipt>, AccountError> {
        let inactive_stake = self
            .inactive_stake_by_address
            .remove(staker_address)
            .ok_or(AccountError::InvalidForSender)?;

        self.balance = Account::balance_sub(self.balance, total_value)?;

        // All checks passed, not allowed to fail from here on!
        if inactive_stake.balance > total_value {
            let new_inactive_stake = InactiveStake {
                balance: Account::balance_sub(inactive_stake.balance, total_value)?,
                retire_time: inactive_stake.retire_time,
            };
            self.inactive_stake_by_address
                .insert(staker_address.clone(), new_inactive_stake);

            Ok(None)
        } else {
            assert_eq!(inactive_stake.balance, total_value);
            Ok(Some(InactiveStakeReceipt {
                retire_time: inactive_stake.retire_time,
            }))
        }
    }

    /// Reverts a unstake transaction.
    pub(super) fn revert_unstake(
        &mut self,
        staker_address: &Address,
        total_value: Coin,
        receipt: Option<InactiveStakeReceipt>,
    ) -> Result<(), AccountError> {
        self.balance = Account::balance_add(self.balance, total_value)?;

        if let Some(inactive_stake) = self.inactive_stake_by_address.remove(staker_address) {
            if receipt.is_some() {
                return Err(AccountError::InvalidReceipt);
            }

            let new_inactive_stake = InactiveStake {
                balance: Account::balance_add(inactive_stake.balance, total_value)?,
                retire_time: inactive_stake.retire_time,
            };
            self.inactive_stake_by_address
                .insert(staker_address.clone(), new_inactive_stake);
        } else {
            let receipt = receipt.ok_or(AccountError::InvalidReceipt)?;
            let new_inactive_stake = InactiveStake {
                balance: total_value,
                retire_time: receipt.retire_time,
            };
            self.inactive_stake_by_address
                .insert(staker_address.clone(), new_inactive_stake);
        }
        Ok(())
    }
}
