use nimiq_account::Accounts;
use nimiq_block::MacroBlock;
use nimiq_hash::Blake3Hash;
use nimiq_primitives::slots::Validators;

use crate::chain_info::ChainInfo;

/// A struct that keeps the current state of the blockchain. It summarizes the information known to
/// a validator at the head of the blockchain.
pub struct BlockchainState {
    // The accounts tree.
    pub accounts: Accounts,
    // The chain info for the head of the main chain.
    pub main_chain: ChainInfo,
    // The hash of the head of the main chain.
    pub head_hash: Blake3Hash,
    // The chain info for the last macro block.
    pub macro_info: ChainInfo,
    // The hash of the last macro block.
    pub macro_head_hash: Blake3Hash,
    // The last election macro block.
    pub election_head: MacroBlock,
    // The hash of the last election macro block.
    pub election_head_hash: Blake3Hash,
    // The validator slots for the current epoch.
    pub current_slots: Option<Validators>,
    // The validator slots for the previous epoch.
    pub previous_slots: Option<Validators>,
}
