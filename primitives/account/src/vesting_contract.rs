use beserial::{Deserialize, Serialize};
use keys::Address;
use primitives::coin::Coin;
use transaction::{SignatureProof, Transaction};
use transaction::account::parse_and_verify_vesting_creation_transaction;

use crate::AccountTransactionInteraction;

use super::{Account, AccountError, AccountType};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct VestingContract {
    pub balance: Coin,
    pub owner: Address,
    pub vesting_start: u32,
    pub vesting_step_blocks: u32,
    pub vesting_step_amount: Coin,
    pub vesting_total_amount: Coin,
}

impl VestingContract {
    pub fn new(balance: Coin, owner: Address, vesting_start: u32, vesting_step_blocks: u32, vesting_step_amount: Coin, vesting_total_amount: Coin) -> Self {
        VestingContract { balance, owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount }
    }

    pub fn with_balance(&self, balance: Coin) -> Self {
        VestingContract {
            balance,
            owner: self.owner.clone(),
            vesting_start: self.vesting_start,
            vesting_step_blocks: self.vesting_step_blocks,
            vesting_step_amount: self.vesting_step_amount,
            vesting_total_amount: self.vesting_total_amount,
        }
    }

    pub fn min_cap(&self, block_height: u32) -> Coin {
        if self.vesting_step_blocks > 0 && self.vesting_step_amount > Coin::ZERO {
            let steps = (f64::from(block_height - self.vesting_start) / f64::from(self.vesting_step_blocks)).floor();
            let min_cap = u64::from(self.vesting_total_amount) as f64 - steps * u64::from(self.vesting_step_amount) as f64;
            Coin::from_u64(min_cap.max(0f64) as u64).unwrap() // Since all parameters have been validated, this will be safe as well.
        } else {
            Coin::ZERO
        }
    }
}

impl AccountTransactionInteraction for VestingContract {
    fn new_contract(account_type: AccountType, balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        if account_type == AccountType::Vesting {
            VestingContract::create(balance, transaction, block_height)
        } else {
            Err(AccountError::InvalidForRecipient)
        }
    }

    fn create(balance: Coin, transaction: &Transaction, _block_height: u32) -> Result<Self, AccountError> {
        let (owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount) = parse_and_verify_vesting_creation_transaction(transaction)?;
        Ok(VestingContract::new(balance, owner, vesting_start, vesting_step_blocks, vesting_step_amount, vesting_total_amount))
    }

    fn check_incoming_transaction(&self, _transaction: &Transaction, _block_height: u32) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn commit_incoming_transaction(&mut self, _transaction: &Transaction, _block_height: u32) -> Result<Option<Vec<u8>>, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn revert_incoming_transaction(&mut self, _transaction: &Transaction, _block_height: u32, _receipt: Option<&Vec<u8>>) -> Result<(), AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn check_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<(), AccountError> {
        // Check vesting min cap.
        let balance: Coin = Account::balance_sub(self.balance, transaction.total_value().ok_or(AccountError::InvalidCoinValue)?)?;
        let min_cap = self.min_cap(block_height);
        if balance < min_cap {
            return Err(AccountError::InsufficientFunds { balance, needed: min_cap });
        }

        // Check transaction signer is contract owner.
        let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;
        if !signature_proof.is_signed_by(&self.owner) {
            return Err(AccountError::InvalidSignature);
        }

        Ok(())
    }

    fn commit_outgoing_transaction(&mut self, transaction: &Transaction, block_height: u32) -> Result<Option<Vec<u8>>, AccountError> {
        self.check_outgoing_transaction(transaction, block_height)?;
        self.balance = Account::balance_sub(self.balance, transaction.total_value().ok_or(AccountError::InvalidCoinValue)?)?;
        Ok(None)
    }

    fn revert_outgoing_transaction(&mut self, transaction: &Transaction, _block_height: u32, receipt: Option<&Vec<u8>>) -> Result<(), AccountError> {
        if receipt.is_some() {
            return Err(AccountError::InvalidForSender);
        }

        self.balance = Account::balance_add(self.balance, transaction.total_value().ok_or(AccountError::InvalidCoinValue)?)?;
        Ok(())
    }
}
