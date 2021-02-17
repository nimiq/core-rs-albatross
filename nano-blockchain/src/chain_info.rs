extern crate nimiq_primitives as primitives;

use nimiq_hash::Blake2bHash;

/// Struct that, for each block, keeps information relative to the chain the block is on.
#[derive(Clone, Debug)]
pub struct ChainInfo {
    // A boolean stating if this block is in the main chain.
    pub on_main_chain: bool,
    // The hash of next block in the chain.
    pub main_chain_successor: Option<Blake2bHash>,
    // The hash of previous block in the chain.
    pub predecessor: Option<Blake2bHash>,
}
