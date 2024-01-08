use nimiq_blockchain_interface::BlockchainError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Blockchain error: {0}")]
    BlockchainError(#[from] BlockchainError),
}

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Other")]
    Other,
    #[error("No valid sync target found")]
    NoValidSyncTarget,
}

/// Different errors that can be obtained when subscribing to transaction addresses.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq, Serialize, Deserialize)]
pub enum SubscribeToAddressesError {
    /// Already attending too many peers
    #[error("Too many peers")]
    TooManyPeers = 1,
    /// Already attending too many addresses
    #[error("Too many addresses")]
    TooManyAddresses = 2,
    /// Some of the provided parameters/operations was invalid
    #[error("Invalid operation")]
    InvalidOperation = 3,
    /// Another type of error not covered by the previous error types
    #[error("Other")]
    #[serde(other)]
    Other = 4,
}
