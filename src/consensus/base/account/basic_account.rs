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
    pub fn verify_incoming_transaction(transaction: &Transaction, block_height: u32) -> bool {
        return true;
    }

    pub fn verify_outgoing_transaction(transaction: &Transaction, block_height: u32) -> bool {
        let proof_buf = &mut &transaction.proof[..];
        let signature_proof: SignatureProof = match Deserialize::deserialize(proof_buf) { Ok(v) => v, Err(e) => return false };
        if signature_proof.compute_signer() != transaction.sender {
            return false;
        }
        return signature_proof.public_key.verify(&signature_proof.signature, transaction.serialize_content().as_slice());
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
