use beserial::{Deserialize, Serialize};

use crate::account::Account;
use crate::account::AccountError;
use crate::coin::Coin;
#[cfg(feature = "transaction")]
use crate::transaction::{Transaction, TransactionError};
#[cfg(feature = "transaction")]
use crate::transaction::SignatureProof;
#[cfg(feature = "transaction")]
use crate::account::AccountTransactionInteraction;
#[cfg(feature = "transaction")]
use crate::account::AccountType;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct BasicAccount {
    pub balance: Coin
}

#[cfg(feature = "transaction")]
impl AccountTransactionInteraction for BasicAccount {
    fn create(balance: Coin, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        Err(AccountError::InvalidForRecipient)
    }

    fn verify_incoming_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        Ok(())
    }

    fn verify_outgoing_transaction(transaction: &Transaction) -> Result<(), TransactionError> {
        let signature_proof: SignatureProof = Deserialize::deserialize(&mut &transaction.proof[..])?;

        if !signature_proof.is_signed_by(&transaction.sender) || !signature_proof.verify(transaction.serialize_content().as_slice()) {
            warn!("Invalid signature");
            return Err(TransactionError::InvalidProof);
        }

        Ok(())
    }

    fn with_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_add(self.balance, transaction.value)?;
        return Ok(BasicAccount { balance });
    }

    fn without_incoming_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.value)?;
        return Ok(BasicAccount { balance });
    }

    fn with_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_sub(self.balance, transaction.value + transaction.fee)?;
        return Ok(BasicAccount { balance });
    }

    fn without_outgoing_transaction(&self, transaction: &Transaction, block_height: u32) -> Result<Self, AccountError> {
        let balance: Coin = Account::balance_add(self.balance, transaction.value + transaction.fee)?;
        return Ok(BasicAccount { balance });
    }
}
