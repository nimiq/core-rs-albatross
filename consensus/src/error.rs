use beserial::{Deserialize, Serialize};
use thiserror::Error;

use nimiq_blockchain_interface::BlockchainError;

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

#[repr(u8)]
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq, Serialize, Deserialize)]
pub enum SubscribebyAdressErrors {
    /// Already attending to many peers
    #[error("TooManyPeers")]
    TooManyPeers = 1,
    /// Already attending to many peers
    #[error("TooManyAddresses")]
    TooManyAddresses = 2,
    /// Some of the provided parameters/operations was invalid
    #[error("InvalidOperation")]
    InvalidOperation = 3,
    /// Another type of error
    #[error("Other")]
    Other = 4,
}
