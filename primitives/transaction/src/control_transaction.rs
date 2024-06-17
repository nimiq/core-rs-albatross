use std::{error::Error, fmt, ops};

use nimiq_primitives::account::AccountType;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};

use crate::Transaction;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ControlTransaction(Transaction);

#[derive(Clone, Debug)]
pub struct ControlTransactionError(Transaction);

impl fmt::Display for ControlTransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "not a control transaction".fmt(f)
    }
}

impl Error for ControlTransactionError {}

impl ControlTransactionError {
    pub fn into_inner(self) -> Transaction {
        self.0
    }
}

impl TryFrom<Transaction> for ControlTransaction {
    type Error = ControlTransactionError;

    fn try_from(tx: Transaction) -> Result<ControlTransaction, ControlTransactionError> {
        if tx.sender_type == AccountType::Staking || tx.recipient_type == AccountType::Staking {
            Ok(ControlTransaction(tx))
        } else {
            Err(ControlTransactionError(tx))
        }
    }
}

impl From<ControlTransaction> for Transaction {
    fn from(tx: ControlTransaction) -> Transaction {
        tx.0
    }
}

impl ops::Deref for ControlTransaction {
    type Target = Transaction;
    fn deref(&self) -> &Transaction {
        &self.0
    }
}

impl Serialize for ControlTransaction {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let tx: &Transaction = self;
        tx.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ControlTransaction {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ControlTransaction, D::Error> {
        let tx = Transaction::deserialize(deserializer)?;
        ControlTransaction::try_from(tx).map_err(D::Error::custom)
    }
}
