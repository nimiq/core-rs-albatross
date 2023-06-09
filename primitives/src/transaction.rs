use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum TransactionError {
    #[error("Transaction is for a foreign network")]
    ForeignNetwork,
    #[error("Transaction has 0 value")]
    ZeroValue,
    #[error("Transaction has invalid value")]
    InvalidValue,
    #[error("Overflow")]
    Overflow,
    #[error("Sender same as recipient")]
    SenderEqualsRecipient,
    #[error("Transaction is invalid for sender")]
    InvalidForSender,
    #[error("Invalid transaction proof")]
    InvalidProof,
    #[error("Transaction is invalid for recipient")]
    InvalidForRecipient,
    #[error("Invalid transaction data")]
    InvalidData,
    #[error("Invalid serialization: {0}")]
    InvalidSerialization(#[from] nimiq_serde::DeserializeError),
}
