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

#[cfg(test)]
mod test {
    use nimiq_keys::Address;
    use nimiq_primitives::{account::AccountType, coin::Coin, networks::NetworkId, policy::Policy};
    use nimiq_serde::{Deserialize, Serialize};

    use super::ControlTransaction;
    use crate::{account::staking_contract::IncomingStakingTransactionData, Transaction};

    fn normal_tx() -> Transaction {
        Transaction::new_basic(
            Address::default(),
            Address::default(),
            Coin::MAX_SAFE_VALUE.try_into().unwrap(),
            Coin::ZERO,
            0,
            NetworkId::UnitAlbatross,
        )
    }

    fn reactivate_tx() -> Transaction {
        Transaction::new_signaling(
            Address::default(),
            AccountType::Basic,
            Policy::STAKING_CONTRACT_ADDRESS,
            AccountType::Staking,
            Coin::ZERO,
            IncomingStakingTransactionData::ReactivateValidator {
                validator_address: Address::default(),
                proof: Default::default(),
            }
            .serialize_to_vec(),
            0,
            NetworkId::UnitAlbatross,
        )
    }

    #[test]
    fn normal_tx_is_not_ctrl() {
        let tx = normal_tx();
        assert_eq!(
            ControlTransaction::try_from(tx.clone())
                .unwrap_err()
                .into_inner(),
            tx,
        );
    }

    #[test]
    fn reactivate_is_ctrl() {
        let tx = reactivate_tx();
        assert_eq!(
            Transaction::from(ControlTransaction::try_from(tx.clone()).unwrap()),
            tx,
        );
    }

    #[test]
    fn normal_tx_deserialize_error() {
        let tx = normal_tx();
        assert!(ControlTransaction::deserialize_from_vec(&tx.serialize_to_vec()).is_err());
    }

    #[test]
    fn reactivate_serialize_roundtrip() {
        let tx = reactivate_tx();
        let ctx = ControlTransaction::try_from(tx.clone()).unwrap();
        assert_eq!(tx.serialize_to_vec(), ctx.serialize_to_vec());
        assert_eq!(
            ControlTransaction::deserialize_from_vec(&tx.serialize_to_vec()).unwrap(),
            ctx,
        );
    }
}
