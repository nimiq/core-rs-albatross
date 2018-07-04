use beserial::{Serialize, Deserialize};
use consensus::base::transaction::Transaction;
use super::{Account, AccountError};

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct HashedTimeLockedContract {
    pub balance: u64
}

impl HashedTimeLockedContract {
    pub fn verify_incoming_transaction(transaction: &Transaction, block_height: u32) -> bool {
        // The contract creation transaction is the only valid incoming transaction.
        return transaction.recipient == transaction.contract_creation_address();
    }

    pub fn verify_outgoing_transaction(transaction: &Transaction, block_height: u32) -> bool {
        // TODO verify signature
        unimplemented!();
    }

    pub fn create(balance: u64, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        unimplemented!();
    }

    pub fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError("Illegal incoming transaction".to_string()));
    }

    pub fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        return Err(AccountError("Illegal incoming transaction".to_string()));
    }

    pub fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: u64 = Account::balance_sub(self.balance, transaction.value + transaction.fee)?;
        return Ok(HashedTimeLockedContract { balance });
    }

    pub fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: u64 = Account::balance_add(self.balance, transaction.value + transaction.fee)?;
        return Ok(HashedTimeLockedContract { balance });
    }
}
