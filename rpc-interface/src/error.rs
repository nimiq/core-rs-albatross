use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid combination of transaction parameters")]
    InvalidTransactionParameters,

    #[error("Invalid block number or hash: {0}")]
    InvalidBlockNumberOrHash(String),
}
