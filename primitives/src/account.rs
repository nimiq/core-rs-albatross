use std::convert::TryFrom;

use strum_macros::Display;
use thiserror::Error;

use beserial::{Deserialize, Serialize, SerializingError};
use nimiq_keys::Address;

use crate::{
    coin::{Coin, CoinConvertError, CoinParseError},
    transaction::TransactionError,
    trie::error::MerkleRadixTrieError,
};

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize, Display)]
#[repr(u8)]
#[cfg_attr(
    feature = "serde-derive",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "u8", into = "u8")
)]
pub enum AccountType {
    Basic = 0,
    Vesting = 1,
    HTLC = 2,
    Staking = 3,
    StakingValidator = 4,
    StakingValidatorsStaker = 5,
    StakingStaker = 6,
}

#[derive(Debug, Error)]
#[error("Can't convert {0} to AccountType.")]
pub struct Error(u8);

impl TryFrom<u8> for AccountType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AccountType::Basic),
            1 => Ok(AccountType::Vesting),
            2 => Ok(AccountType::HTLC),
            3 => Ok(AccountType::Staking),
            4 => Ok(AccountType::StakingValidator),
            5 => Ok(AccountType::StakingValidatorsStaker),
            6 => Ok(AccountType::StakingStaker),
            _ => Err(Error(value)),
        }
    }
}

impl From<AccountType> for u8 {
    fn from(ty: AccountType) -> Self {
        match ty {
            AccountType::Basic => 0,
            AccountType::Vesting => 1,
            AccountType::HTLC => 2,
            AccountType::Staking => 3,
            AccountType::StakingValidator => 4,
            AccountType::StakingValidatorsStaker => 5,
            AccountType::StakingStaker => 6,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AccountError {
    #[error("Insufficient funds: needed {needed}, but has balance {balance}")]
    InsufficientFunds { needed: Coin, balance: Coin },
    #[error("Type mismatch: expected {expected}, but got {got}")]
    TypeMismatch {
        expected: AccountType,
        got: AccountType,
    },
    #[error("Invalid signature")]
    InvalidSignature,
    #[error("Invalid for sender")]
    InvalidForSender,
    #[error("Invalid for recipient")]
    InvalidForRecipient,
    #[error("Invalid for target")]
    InvalidForTarget,
    #[error("Invalid receipt")]
    InvalidReceipt,
    #[error("Invalid serialization")]
    InvalidSerialization(#[from] SerializingError),
    #[error("Invalid transaction")]
    InvalidTransaction(#[from] TransactionError),
    #[error("Invalid coin value")]
    InvalidCoinValue,
    #[error("Invalid coin value: {0}")]
    CoinParse(#[from] CoinParseError),
    #[error("Invalid coin value: {0}")]
    CoinConvert(#[from] CoinConvertError),
    #[error("Invalid inherent")]
    InvalidInherent,
    #[error("Address {address} does not exist in the Accounts Tree.")]
    NonExistentAddress { address: Address },
    #[error("There is already an account at address {address} in the Accounts Tree.")]
    AlreadyExistentAddress { address: Address },
    #[error("Error during chunk processing: {0}")]
    ChunkError(#[from] MerkleRadixTrieError),
}
