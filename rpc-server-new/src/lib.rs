pub mod types;
pub(crate) mod serde_helpers;
pub mod dispatchers;
pub mod wallets;

use std::{
    collections::HashSet,
    net::SocketAddr,
    fmt::{Display, Formatter},
};

use nimiq_jsonrpc_core::RpcError;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
pub use nimiq_jsonrpc_server::{Config, Server};

use thiserror::Error;
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use crate::types::OrLatest;


#[derive(Clone, Debug)]
pub enum BlockNumberOrHash {
    Number(u32),
    Hash(Blake2bHash),
}

impl From<u32> for BlockNumberOrHash {
    fn from(block_number: u32) -> Self {
        BlockNumberOrHash::Number(block_number)
    }
}

impl From<Blake2bHash> for BlockNumberOrHash {
    fn from(block_hash: Blake2bHash) -> Self {
        BlockNumberOrHash::Hash(block_hash)
    }
}

impl Display for BlockNumberOrHash {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        match self {
            BlockNumberOrHash::Number(block_number) => write!(f, "{}", block_number),
            BlockNumberOrHash::Hash(block_hash) => write!(f, "{}", block_hash),
        }
    }
}


#[derive(Debug, Error)]
pub enum Error {
    #[error("Block not found: {0}")]
    BlockNotFound(BlockNumberOrHash),

    #[error("Unexpected macro block: {0}")]
    UnexpectedMacroBlock(BlockNumberOrHash),

    #[error("Method not implemented")]
    NotImplemented,

    #[error("Invalid combination of transaction parameters")]
    InvalidTransactionParameters,

    #[error("No account with address: {0}")]
    AccountNotFound(Address),

    #[error("Wrong passphrase")]
    WrongPassphrase,

    #[error("No unlocked wallet with address: {0}")]
    UnlockedWalletNotFound(Address),

    #[error("Invalid hex: {0}")]
    HexError(#[from] hex::FromHexError),

    #[error("{0}")]
    Beserial(#[from] beserial::SerializingError),

    #[error("{0}")]
    Argon2(#[from] nimiq_hash::argon2kdf::Argon2Error),

    #[error("Transaction rejected: {0:?}")]
    TransactionRejected(nimiq_mempool::ReturnCode),
}


impl From<Error> for nimiq_jsonrpc_core::RpcError {
    fn from(e: Error) -> Self {
        // TODO
        RpcError::internal_error(Some(e.to_string()))
    }
}
