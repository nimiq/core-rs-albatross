use nimiq_accounts_tree::Accounts;
use nimiq_block::MacroBlock;
use nimiq_blockchain_interface::ChainInfo;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::slots::Validators;

/// A struct that keeps the current state of the blockchain. It summarizes the information known to
/// a validator at the head of the blockchain.
pub struct BlockchainState {
    /// The accounts tree.
    pub accounts: Accounts,
    /// The chain info for the head of the main chain.
    pub main_chain: ChainInfo,
    /// The hash of the head of the main chain.
    pub head_hash: Blake2bHash,
    /// The chain info for the last macro block.
    pub macro_info: ChainInfo,
    /// The hash of the last macro block.
    pub macro_head_hash: Blake2bHash,
    /// The last election macro block.
    pub election_head: MacroBlock,
    /// The hash of the last election macro block.
    pub election_head_hash: Blake2bHash,
    /// The validator slots for the current epoch.
    pub current_slots: Option<Validators>,
    /// The validator slots for the previous epoch.
    pub previous_slots: Option<Validators>,
    /// During syncing it can happen that we don't have the full transaction history for the full epoch;
    /// thus we cannot verify the history root.
    pub can_verify_history: bool,
}
