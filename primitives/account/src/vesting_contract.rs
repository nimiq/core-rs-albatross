use beserial::{Deserialize, Serialize};
use nimiq_keys::Address;
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::Coin;
use nimiq_transaction::account::vesting_contract::CreationTransactionData;
use nimiq_transaction::{SignatureProof, Transaction};

use crate::inherent::Inherent;
use crate::interaction_traits::{AccountInherentInteraction, AccountTransactionInteraction};
use crate::{Account, AccountError};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "serde-derive", derive(serde::Serialize, serde::Deserialize))]
pub struct VestingContract {
    pub balance: Coin,
    pub owner: Address,
    pub start_time: u64,
    pub time_step: u64,
    pub step_amount: Coin,
    pub total_amount: Coin,
}

impl VestingContract {
    pub fn new(
        balance: Coin,
        owner: Address,
        start_time: u64,
        time_step: u64,
        step_amount: Coin,
        total_amount: Coin,
    ) -> Self {
        VestingContract {
            balance,
            owner,
            start_time,
            time_step,
            step_amount,
            total_amount,
        }
    }

    pub fn with_balance(&self, balance: Coin) -> Self {
        VestingContract {
            balance,
            owner: self.owner.clone(),
            start_time: self.start_time,
            time_step: self.time_step,
            step_amount: self.step_amount,
            total_amount: self.total_amount,
        }
    }

    pub fn min_cap(&self, time: u64) -> Coin {
        if self.time_step > 0 && self.step_amount > Coin::ZERO {
            let steps = (time as i128 - self.start_time as i128) / self.time_step as i128;
            let min_cap =
                u64::from(self.total_amount) as i128 - steps * u64::from(self.step_amount) as i128;
            // Since all parameters have been validated, this will be safe as well.
            Coin::from_u64_unchecked(min_cap.max(0) as u64)
        } else {
            Coin::ZERO
        }
    }
}

impl AccountTransactionInteraction for VestingContract {
    fn new_contract(
        account_type: AccountType,
        balance: Coin,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Self, AccountError> {
        if account_type == AccountType::Vesting {
            VestingContract::create(balance, transaction, block_height, time)
        } else {
            Err(AccountError::InvalidForRecipient)
        }
    }

    fn create(
        balance: Coin,
        transaction: &Transaction,
        _block_height: u32,
        _time: u64,
    ) -> Result<Self, AccountError> {
        let data = CreationTransactionData::parse(transaction)?;
        Ok(VestingContract::new(
            balance,
            data.owner,
            data.start_time,
            data.time_step,
            data.step_amount,
            data.total_amount,
        ))
    }

    fn check_incoming_transaction(
        _transaction: &Transaction,
        _block_height: u32,
        _time: u64,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_height: u32,
        _time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_incoming_transaction(
        &mut self,
        _transaction: &Transaction,
        _block_height: u32,
        _time: u64,
        _receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn check_outgoing_transaction(
        &self,
        transaction: &Transaction,
        _block_height: u32,
        time: u64,
    ) -> Result<(), AccountError> {
        // Check vesting min cap.
        let balance: Coin = Account::balance_sub(self.balance, transaction.total_value()?)?;
        let min_cap = self.min_cap(time);
        if balance < min_cap {
            return Err(AccountError::InsufficientFunds {
                balance,
                needed: min_cap,
            });
        }

        // Check transaction signer is contract owner.
        let signature_proof: SignatureProof =
            Deserialize::deserialize(&mut &transaction.proof[..])?;
        if !signature_proof.is_signed_by(&self.owner) {
            return Err(AccountError::InvalidSignature);
        }

        Ok(())
    }

    fn commit_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        block_height: u32,
        time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        self.check_outgoing_transaction(transaction, block_height, time)?;
        self.balance = Account::balance_sub(self.balance, transaction.total_value()?)?;
        Ok(None)
    }

    fn revert_outgoing_transaction(
        &mut self,
        transaction: &Transaction,
        _block_height: u32,
        _time: u64,
        receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidReceipt);
        }

        self.balance = Account::balance_add(self.balance, transaction.total_value()?)?;
        Ok(())
    }
}

impl AccountInherentInteraction for VestingContract {
    fn check_inherent(
        &self,
        _inherent: &Inherent,
        _block_height: u32,
        _time: u64,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidInherent)
    }

    fn commit_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_height: u32,
        _time: u64,
    ) -> Result<Option<Vec<u8>>, AccountError> {
        Err(AccountError::InvalidInherent)
    }

    fn revert_inherent(
        &mut self,
        _inherent: &Inherent,
        _block_height: u32,
        _time: u64,
        _receipt: Option<&Vec<u8>>,
    ) -> Result<(), AccountError> {
        Err(AccountError::InvalidInherent)
    }
}
