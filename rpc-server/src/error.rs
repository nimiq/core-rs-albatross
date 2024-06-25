use nimiq_hash::Blake2bHash;
use nimiq_jsonrpc_core::RpcError;
use nimiq_keys::Address;
use nimiq_mempool::verify::VerifyErr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Core(#[from] nimiq_rpc_interface::error::Error),

    #[error("{0}")]
    NetworkError(#[from] nimiq_network_libp2p::NetworkError),

    #[error("Mempool rejected transaction: {0}")]
    MempoolError(VerifyErr),

    #[error("Block not found: {0}")]
    BlockNotFound(u32),

    #[error("Block not found: {0}")]
    BlockNotFoundByHash(Blake2bHash),

    #[error("Block number cannot be smaller than genesis block")]
    BlockNumberBeforeGenesis,

    #[error("Unexpected macro block: {0}")]
    UnexpectedMacroBlock(u32),

    #[error("Unexpected macro block: {0}")]
    UnexpectedMacroBlockByHash(Blake2bHash),

    #[error("Method not implemented")]
    NotImplemented,

    #[error("Method not supported for a light blockchain")]
    NotSupportedForLightBlockchain,

    #[error("Method requires a history index")]
    RequiresHistoryIndex,

    #[error("Invalid combination of transaction parameters")]
    InvalidTransactionParameters,

    #[error("Failed to build a transaction: {0}")]
    TransactionBuilder(#[from] nimiq_transaction_builder::TransactionBuilderError),

    #[error("No account with address: {0}")]
    AccountNotFound(Address),

    #[error("No validator with address: {0}")]
    ValidatorNotFound(Address),

    #[error("Validator with address {0} is already {1}")]
    ValidatorAlreadyInState(Address, String),

    #[error("Validator with address {0} is retired thus cannot be reactivated")]
    ValidatorRetired(Address),

    #[error("No staker with address: {0}")]
    StakerNotFound(Address),

    #[error("Wrong passphrase")]
    WrongPassphrase,

    #[error("No unlocked wallet with address: {0}")]
    UnlockedWalletNotFound(Address),

    #[error("Invalid hex: {0}")]
    HexError(#[from] hex::FromHexError),

    #[error("Serialization error: {0}")]
    Serialization(#[from] nimiq_serde::DeserializeError),

    #[error("{0}")]
    Argon2(#[from] nimiq_hash::argon2kdf::Argon2Error),

    #[error("Transaction not found: {0}")]
    TransactionNotFound(Blake2bHash),

    #[error("Multiple transactions found: {0}")]
    MultipleTransactionsFound(Blake2bHash),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("No consensus")]
    NoConsensus,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<Error> for RpcError {
    fn from(e: Error) -> Self {
        RpcError::internal_error(Some(serde_json::value::Value::String(e.to_string())))
    }
}
