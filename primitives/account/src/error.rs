use thiserror::Error;

use beserial::SerializingError;
use nimiq_keys::Address;
use nimiq_primitives::account::AccountType;
use nimiq_primitives::coin::{Coin, CoinConvertError, CoinParseError};
use nimiq_transaction::TransactionError;

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
    #[error("There is already a contract at address {address} in the Accounts Tree.")]
    AlreadyExistentContract { address: Address },
}
