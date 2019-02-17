use failure::Fail;

use network_primitives::networks::NetworkId;

#[derive(Debug, Fail, Clone, PartialEq, Eq)]
pub enum BlockchainError {
    #[fail(display = "Invalid genesis block stored. Are you on the right network?")]
    InvalidGenesisBlock,
    #[fail(display = "Failed to load the main chain. Reset your consensus database.")]
    FailedLoadingMainChain,
    #[fail(display = "Inconsistent chain/accounts state. Reset your consensus database.")]
    InconsistentState,
    #[fail(display = "No network for: {:?}", _0)]
    NoNetwork(NetworkId),
}
