use nimiq_block_albatross::{Block, BlockType, MacroBlock};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy;
use nimiq_primitives::slots::Validators;

use crate::{Blockchain, ChainInfo};
use nimiq_database::Transaction;

/// Defines several basic methods for blockchains.
pub trait AbstractBlockchain {
    /// Returns the head of the main chain.
    fn head(&self) -> Block;

    /// Returns the last macro block.
    fn macro_head(&self) -> Block;

    /// Returns the last election macro block.
    fn election_head(&self) -> MacroBlock;

    /// Returns the hash of the head of the main chain.
    fn head_hash(&self) -> Blake2bHash;

    /// Returns the hash of the last macro block.
    fn macro_head_hash(&self) -> Blake2bHash;

    /// Returns the hash of the last election macro block.
    fn election_head_hash(&self) -> Blake2bHash;

    /// Returns the block number at the head of the main chain.
    fn block_number(&self) -> u32;

    /// Returns the timestamp at the head of the main chain.
    fn timestamp(&self) -> u64;

    /// Returns the view number at the head of the main chain.
    fn view_number(&self) -> u32;

    /// Returns the next view number at the head of the main chain.
    fn next_view_number(&self) -> u32;

    /// Returns the block type of the next block.
    fn get_next_block_type(&self, last_number: Option<u32>) -> BlockType {
        let last_block_number = match last_number {
            None => self.block_number(),
            Some(n) => n,
        };

        if policy::is_macro_block_at(last_block_number + 1) {
            BlockType::Macro
        } else {
            BlockType::Micro
        }
    }

    /// Returns the current set of validators.
    fn current_validators(&self) -> Option<Validators>;

    /// Returns the set of validators of the previous epoch.
    fn previous_validators(&self) -> Option<Validators>;

    /// Checks if the blockchain contains a specific block, by its hash.
    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool;

    /// Fetches a given block, by its block number.
    fn get_block_at(
        &self,
        height: u32,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Block>;

    /// Fetches a given block, by its hash.
    fn get_block(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Block>;

    /// Fetches a given chain info, by its hash.
    fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<ChainInfo>;
}

impl AbstractBlockchain for Blockchain {
    fn head(&self) -> Block {
        self.state.read().main_chain.head.clone()
    }

    fn macro_head(&self) -> Block {
        self.state.read().macro_info.head.clone()
    }

    fn election_head(&self) -> MacroBlock {
        self.state.read().election_head.clone()
    }

    fn head_hash(&self) -> Blake2bHash {
        self.state.read().head_hash.clone()
    }

    fn macro_head_hash(&self) -> Blake2bHash {
        self.state.read().macro_head_hash.clone()
    }

    fn election_head_hash(&self) -> Blake2bHash {
        self.state.read().election_head_hash.clone()
    }

    fn block_number(&self) -> u32 {
        self.state.read_recursive().main_chain.head.block_number()
    }

    fn timestamp(&self) -> u64 {
        self.state.read_recursive().main_chain.head.timestamp()
    }

    fn view_number(&self) -> u32 {
        self.state.read_recursive().main_chain.head.view_number()
    }

    fn next_view_number(&self) -> u32 {
        self.state
            .read_recursive()
            .main_chain
            .head
            .next_view_number()
    }

    fn current_validators(&self) -> Option<Validators> {
        self.state.read().current_slots.clone()
    }

    fn previous_validators(&self) -> Option<Validators> {
        self.state.read().previous_slots.clone()
    }

    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool {
        match self.chain_store.get_chain_info(hash, false, None) {
            Some(chain_info) => include_forks || chain_info.on_main_chain,
            None => false,
        }
    }

    fn get_block_at(
        &self,
        height: u32,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Block> {
        self.chain_store
            .get_block_at(height, include_body, txn_option)
    }

    fn get_block(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Block> {
        self.chain_store.get_block(hash, include_body, txn_option)
    }

    fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<ChainInfo> {
        self.chain_store
            .get_chain_info(hash, include_body, txn_option)
    }
}
