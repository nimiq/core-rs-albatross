use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid combination of transaction parameters")]
    InvalidTransactionParameters,

    #[error("Invalid block number or hash: {0}")]
    InvalidBlockNumberOrHash(String),

    #[error("Invalid log type: {0}")]
    InvalidLogType(String),

    // This is likely unreachable!() due to the nature of staking contract internal account types,
    // but is added for completeness.
    // Getting rid of staking contract internal account types like StakingStaker etc makes this obsolete.
    #[error("Unsupported account type")]
    UnsupportedAccountType,
}
