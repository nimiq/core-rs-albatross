use std::process::ExitStatus;

use hex::FromHexError;
use nimiq_genesis_builder::config::{
    GenesisAccount, GenesisHTLC, GenesisStaker, GenesisVestingContract,
};
use nimiq_keys::{Address, AddressParseError};
use nimiq_primitives::{
    coin::{Coin, CoinConvertError},
    networks::NetworkId,
};
use thiserror::Error;

/// PoW block registration window
///
/// The registration window is a set of blocks in the PoW chain that marks
/// the start and end of different windows as follows:
///
/// ```text
///
///     1              2     3              4     5     6         7          8
/// --- | ------------ | --- | ------------ | --- | --- | ------- | -------- | ---
///
/// ```
///
/// 1. Validator registration window start block.
/// 2. Validator registration window end block.
/// 3. Pre-stake registration window start.
/// 4. Pre-stake registration window end block.
/// 5. The transition candidate block in the PoW chain that will be taken as genesis
///    block for the PoS chain.
/// 6. This is a block whose block number is a number of confirmations away from
///    the final block described in 5.
/// 7. Marks the end of an activation window. If not enough validators signaled readiness
///    within blocks 5 and 7, then another window is used (between blocks 7 and 8) and 7
///    becomes now the new transition candidate block. This process would repeat until
///    reaching enough validators signaling readiness.
#[derive(Debug)]
pub struct BlockWindows {
    /// Block number of the validator registration window start.
    pub registration_start: u32,
    /// Block number of the validator registration window wnd.
    pub registration_end: u32,
    /// Block number of the pre stake registration window start.
    pub pre_stake_start: u32,
    /// Block number of the pre stake registration window end.
    pub pre_stake_end: u32,

    /// The first block from the PoW that is used to create the first PoS candidate block.
    pub election_candidate: u32,

    /// Number of confirmations after any block is considered final in the PoW chain.
    pub block_confirmations: u32,

    /// If not enough validators are ready to start the PoS chain at the election candidate,
    /// a new candidate is elected after readiness_window blocks.
    /// This process is repeated until we start the PoS chain.
    pub readiness_window: u32,
}

/// PoS agents that were registered in the PoW chain that will take part of the
/// PoS genesis block.
pub struct PoSRegisteredAgents {
    /// Registered PoS validators
    pub validators: Vec<GenesisValidator>,
    /// Registered PoS stakers
    pub stakers: Vec<GenesisStaker>,
}

/// Genesis accounts for the genesis state
#[derive(Debug)]
pub struct GenesisAccounts {
    /// Basic accounts for the genesis state.
    pub basic_accounts: Vec<GenesisAccount>,

    /// Vesting accounts for the genesis state.
    pub vesting_accounts: Vec<GenesisVestingContract>,

    /// HTLC accounts for the genesis state.
    pub htlc_accounts: Vec<GenesisHTLC>,
}

/// Genesis validators for the genesis state
#[derive(Clone, Debug)]
pub struct GenesisValidator {
    /// Inner genesis validator information
    pub validator: nimiq_genesis_builder::config::GenesisValidator,

    /// Validator stake
    pub total_stake: Coin,
}

/// Error types that can be returned
#[derive(Error, Debug)]
pub enum StateError {
    /// RPC error
    #[error("RPC error: {0}")]
    Rpc(#[from] nimiq_rpc::jsonrpsee::core::ClientError),
    /// Address parsing error
    #[error("Failed to parse Nimiq address: {0}")]
    Address(#[from] AddressParseError),
    /// Coin conversion error
    #[error("Failed to convert to coin: {0}")]
    Coin(#[from] CoinConvertError),
    /// Hex conversion error
    #[error("Failed to decode string as hex: {0}")]
    Hex(#[from] FromHexError),
    /// Invalid value
    #[error("Invalid value")]
    InvalidValue,
    /// Invalid Genesis timestamp
    #[error("Invalid genesis timestamp: {0}")]
    InvalidTimestamp(u64),
    /// RPC server not ready
    #[error("RPC server is not ready")]
    RPCServerNotReady,
}

/// Error types that can be returned
#[derive(Error, Debug)]
pub enum GenesisError {
    /// RPC error
    #[error("RPC error: {0}")]
    Rpc(#[from] nimiq_rpc::jsonrpsee::core::ClientError),
    /// Unknown PoW block
    #[error("Unknown PoW block")]
    UnknownBlock,
    /// State migration error
    #[error("State migration error: {0}")]
    State(#[from] StateError),
    /// Hex conversion error
    #[error("Failed to decode string as hex")]
    Hex(#[from] FromHexError),
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
    InvalidNetworkId(NetworkId),
}

/// Error types that can be returned
#[derive(Error, Debug)]
pub enum HistoryError {
    /// RPC error
    #[error("RPC error: {0}")]
    Rpc(#[from] nimiq_rpc::jsonrpsee::core::ClientError),
    /// Unknown PoW block
    #[error("Unknown PoW block")]
    UnknownBlock,
    /// Address parsing error
    #[error("Failed to parse Nimiq address")]
    Address(#[from] AddressParseError),
    /// Coin conversion error
    #[error("Failed to convert to coin")]
    Coin(#[from] CoinConvertError),
    /// Hex decoding error
    #[error("Failed to decode HEX string")]
    Hex(#[from] FromHexError),
    /// Invalid value
    #[error("Invalid value")]
    InvalidValue,
    /// Error calculating history root
    #[error("History root error")]
    HistoryRootError,
}

/// Error types that can be returned
#[derive(Error, Debug)]
pub enum Error {
    /// Invalid Network ID
    #[error("Invalid Network ID")]
    InvalidNetworkID(NetworkId),
    /// PoS client unexpectedly exited
    #[error("PoS client unexpectedly exited with status: {0}")]
    PoSUnexpectedExit(ExitStatus),
    /// I/O error
    #[error("I/O error: {0}")]
    IO(#[from] std::io::Error),
    /// Genesis building error
    #[error("Error building genesis: {0}")]
    Genesis(#[from] GenesisError),
    /// State migration error
    #[error("State migration error: {0}")]
    State(#[from] StateError),
    /// Migration monitor error
    #[error("Migration monitor error: {0}")]
    Monitor(#[from] crate::monitor::Error),
    /// History migration error
    #[error("History migration error: {0}")]
    History(#[from] HistoryError),
    /// Validator key hasn't been imported
    #[error("Validator key hasn't been imported: {0}")]
    ValidatorKey(Address),
}
