pub mod genesis;
pub mod history;
pub mod monitor;
pub mod state;

use nimiq_primitives::networks::NetworkId;
use thiserror::Error;

static TESTNET_BLOCK_WINDOWS: &BlockWindows = &BlockWindows {
    registration_start: 2590000,
    registration_end: 2660000,
    pre_stake_start: 2660000,
    pre_stake_end: 2663100,
    block_confirmations: 10,
};

// Currently not used because there is still no network ID for PoS MainNet
#[allow(dead_code)]
static MAINNET_BLOCK_WINDOWS: &BlockWindows = &BlockWindows {
    registration_start: 2590000,
    registration_end: 2660000,
    pre_stake_start: 2660000,
    pre_stake_end: 2663100,
    block_confirmations: 10,
};

/// PoW block registration window
///
/// The registration window is a set of blocks in the PoW chain that marks
/// the start and end of different windows as follows:
///
///     1              2              3              4              5        6
/// --- | ------------ | ------------ | ------------ | ------------ |------- |
///
/// 1. Validator registration window start block.
/// 2. Validator registration window end block.
/// 3. Pre-stake registration window start.
/// 4. Pre-stake registration window end block. This block is also the activation
///    window start.
/// 5. The final block in the PoW chain that will be taken as genesis block for the
///    PoS chain. This block must have a block number that can be an election block
///    number in the PoS chain.
/// 6. This is a block whose block number is a number of confirmations away from
///    the final block described in 4.
///
#[derive(Debug)]
pub struct BlockWindows {
    /// Block number of the validator registration window start.
    pub registration_start: u32,
    /// Block number of the validator registration window wnd.
    pub registration_end: u32,
    /// Block number of the validator registration window end which is also
    /// the pre stake registration window start.
    pub pre_stake_start: u32,
    /// Block number of the pre stake registration window end.
    pub pre_stake_end: u32,
    /// Number of confirmations after the final block needed for the PoS chain to
    /// start.
    pub block_confirmations: u32,
}

/// Error types that can be returned
#[derive(Error, Debug)]
pub enum Error {
    /// Invalid Network ID
    #[error("Invalid Network ID")]
    InvalidNetworkID(NetworkId),
}

pub fn get_block_windows(network_id: NetworkId) -> Result<&'static BlockWindows, Error> {
    match network_id {
        NetworkId::TestAlbatross => Ok(TESTNET_BLOCK_WINDOWS),
        _ => Err(Error::InvalidNetworkID(network_id)),
    }
}
