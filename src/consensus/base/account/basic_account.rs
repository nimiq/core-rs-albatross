use beserial::{Serialize, Deserialize};
use super::AccountError;
use super::super::transaction::{Transaction, TransactionError};
use crate::consensus::base::account::Account;
use crate::consensus::base::transaction::SignatureProof;
use crate::consensus::base::primitive::Coin;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct BasicAccount {
    pub balance: Coin
}

impl BasicAccount {
    pub fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        Ok(())
    }

    pub fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;

        if !signature_proof.is_signed_by(&transaction.sender) || !signature_proof.verify(transaction.serialize_content().as_slice()) {
            warn!("Invalid signature");
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }

    pub fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_add(self.balance, transaction.value)?;
        return Ok(BasicAccount { balance });
    }

    pub fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.value)?;
        return Ok(BasicAccount { balance });
    }

    pub fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.value + transaction.fee)?;
        return Ok(BasicAccount { balance });
    }

    pub fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_add(self.balance, transaction.value + transaction.fee)?;
        return Ok(BasicAccount { balance });
    }
}
