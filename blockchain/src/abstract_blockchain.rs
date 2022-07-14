use nimiq_block::{Block, BlockType, MacroBlock};
use nimiq_database::Transaction;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy::Policy;
use nimiq_primitives::slots::{Validator, Validators};

use crate::{Blockchain, ChainInfo};

/// Defines several basic methods for blockchains.
pub trait AbstractBlockchain {
    /// Returns the network id.
    fn network_id(&self) -> NetworkId;

    /// Returns the current time.
    fn now(&self) -> u64;

    /// Returns the head of the main chain.
    fn head(&self) -> Block;

    /// Returns the last macro block.
    fn macro_head(&self) -> MacroBlock;

    /// Returns the last election macro block.
    fn election_head(&self) -> MacroBlock;

    /// Returns the hash of the head of the main chain.
    fn head_hash(&self) -> Blake2bHash {
        self.head().hash()
    }

    /// Returns the hash of the last macro block.
    fn macro_head_hash(&self) -> Blake2bHash {
        self.macro_head().hash()
    }

    /// Returns the hash of the last election macro block.
    fn election_head_hash(&self) -> Blake2bHash {
        self.election_head().hash()
    }

    /// Returns the block number at the head of the main chain.
    fn block_number(&self) -> u32 {
        self.head().block_number()
    }

    /// Returns the epoch number at the head of the main chain.
    fn epoch_number(&self) -> u32 {
        self.head().epoch_number()
    }

    /// Returns the timestamp at the head of the main chain.
    fn timestamp(&self) -> u64 {
        self.head().timestamp()
    }

    /// Returns the block type of the next block.
    fn get_next_block_type(&self, last_number: Option<u32>) -> BlockType {
        let last_block_number = match last_number {
            None => self.block_number(),
            Some(n) => n,
        };

        if Policy::is_macro_block_at(last_block_number + 1) {
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

    /// Calculates the slot owner (represented as the validator plus the slot number) at a given
    /// block number and offset
    fn get_slot_owner_at(
        &self,
        block_number: u32,
        offset: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<(Validator, u16)>;
}

impl AbstractBlockchain for Blockchain {
    fn network_id(&self) -> NetworkId {
        self.network_id
    }

    fn now(&self) -> u64 {
        self.time.now()
    }

    fn head(&self) -> Block {
        self.state.main_chain.head.clone()
    }

    fn macro_head(&self) -> MacroBlock {
        self.state.macro_info.head.unwrap_macro_ref().clone()
    }

    fn election_head(&self) -> MacroBlock {
        self.state.election_head.clone()
    }

    fn block_number(&self) -> u32 {
        self.state.main_chain.head.block_number()
    }

    fn epoch_number(&self) -> u32 {
        self.state.main_chain.head.epoch_number()
    }

    fn current_validators(&self) -> Option<Validators> {
        self.state.current_slots.clone()
    }

    fn previous_validators(&self) -> Option<Validators> {
        self.state.previous_slots.clone()
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

    fn get_slot_owner_at(
        &self,
        block_number: u32,
        offset: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<(Validator, u16)> {
        let vrf_entropy = self
            .get_block_at(block_number - 1, false, txn_option)?
            .seed()
            .entropy();
        self.get_proposer_at(block_number, offset, vrf_entropy, txn_option)
            .map(|slot| (slot.validator, slot.number))
    }
}
