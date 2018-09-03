use beserial::{Serialize, Deserialize};
use super::AccountError;
use super::super::transaction::Transaction;
use consensus::base::account::Account;
use consensus::base::transaction::SignatureProof;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct BasicAccount {
    pub balance: u64
}

impl BasicAccount {
    pub fn verify_incoming_transaction(transaction: &Transaction) -> bool {
        return true;
    }

    pub fn verify_outgoing_transaction(transaction: &Transaction) -> bool {
        let signature_proof: SignatureProof = match Deserialize::deserialize(&mut &transaction.proof[..]) {
            Ok(v) => v,
            Err(e) => return false
        };
        return signature_proof.is_signed_by(&transaction.sender)
            && signature_proof.verify(transaction.serialize_content().as_slice());
    }

    pub fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: u64 = Account::balance_add(self.balance, transaction.value)?;
        return Ok(BasicAccount { balance });
    }

    pub fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: u64 = Account::balance_sub(self.balance, transaction.value)?;
        return Ok(BasicAccount { balance });
    }

    pub fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: u64 = Account::balance_sub(self.balance, transaction.value + transaction.fee)?;
        return Ok(BasicAccount { balance });
    }

    pub fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: u64 = Account::balance_add(self.balance, transaction.value + transaction.fee)?;
        return Ok(BasicAccount { balance });
    }
}
