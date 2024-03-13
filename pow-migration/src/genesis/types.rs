use nimiq_genesis_builder::config::GenesisStaker;
use thiserror::Error;

use crate::state::types::GenesisValidator;

/// Error types that can be returned
#[derive(Error, Debug)]
pub enum Error {
    /// RPC error
    #[error("RPC error: {0}")]
    Rpc(#[from] nimiq_rpc::jsonrpsee::core::ClientError),
    /// Unknown PoW block
    #[error("Unknown PoW block")]
    UnknownBlock,
    /// State migration error
    #[error("State migration error: {0}")]
    State(#[from] crate::state::types::Error),
    /// Hex conversion error
    #[error("Failed to decode string as hex")]
    Hex(#[from] hex::FromHexError),
    /// Serialization error
    #[error("Serialization: {0}")]
    Serialization(#[from] toml::ser::Error),
    /// Invalid time
    #[error("Invalid timestamp")]
    Timestamp(#[from] time::error::ComponentRange),
    /// IO error
    #[error("I/O error: {0}")]
    IO(#[from] std::io::Error),
    /// Invalid Network ID
    #[error("Invalid network ID {0}")]
    InvalidNetworkId(nimiq_primitives::networks::NetworkId),
}

/// PoW registration window
///
/// The registration window is a set of blocks in the PoW chain that marks
/// the start and end of different windows as follows:
///
/// ```text
///     1                  2                  3                  4         5
/// --- | ---------------- | ---------------- | ---------------- | ------- |
///
/// ```
///
/// 1. Validator registration window start block.
/// 2. Validator registration window end block. This block is also the pre-stake
///    registration window start.
/// 3. Pre-stake registration window end block. This block is also the activation
///    window start.
/// 4. The final block in the PoW chain that will be taken as genesis block for the
///    PoS chain. This block must have a block number that can be an election block
///    number in the PoS chain.
/// 5. This is a block whose block number is a number of confirmations away from
///    the final block described in 4.
///
#[derive(Debug)]
pub struct PoWRegistrationWindow {
    /// Block number of the validator registration window start
    pub validator_start: u32,
    /// Block number of the validator registration window end which is also
    /// the pre stake registration window start.
    pub pre_stake_start: u32,
    /// Block number of the pre stake registration window end.
    pub pre_stake_end: u32,
    /// Final block hash in the PoW chain that will be taken as the genesis block
    /// for the PoS chain.
    pub final_block: String,
    /// Number of confirmations after the final block needed for the PoS chain to
    /// start.
    pub confirmations: u32,
}

/// PoS agents that were registered in the PoW chain that will take part of the
/// PoS genesis block.
pub struct PoSRegisteredAgents {
    /// Registered PoS validators
    pub validators: Vec<GenesisValidator>,
    /// Registered PoS stakers
    pub stakers: Vec<GenesisStaker>,
}
