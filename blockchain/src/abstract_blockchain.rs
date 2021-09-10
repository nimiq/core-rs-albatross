use nimiq_block::{Block, BlockType, MacroBlock};
use nimiq_collections::BitSet;
use nimiq_database::Transaction;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy;
use nimiq_primitives::slots::{Validator, Validators};
use nimiq_vrf::{Rng, VrfUseCase};

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

    /// Returns the view number at the head of the main chain.
    fn view_number(&self) -> u32 {
        self.head().view_number()
    }

    /// Returns the next view number at the head of the main chain.
    fn next_view_number(&self) -> u32 {
        self.head().next_view_number()
    }

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

    /// Calculates the slot owner (represented as the validator plus the slot number) at a given
    /// block number and view number.
    fn get_slot_owner_at(
        &self,
        block_number: u32,
        view_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<(Validator, u16)> {
        // Get the disabled slots for the current batch.
        let disabled_slots = self
            .get_block_at(policy::macro_block_before(block_number), true, txn_option)?
            .unwrap_macro()
            .body
            .unwrap()
            .disabled_set;

        // Get the slot number for the current block.
        let slot_number =
            self.get_slot_owner_number_at(block_number, view_number, disabled_slots, txn_option);

        // Get the current validators.
        // Note: We need to handle the case where `block_number()` is at an election block
        // (so `current_slots()` was already updated by it, pushing this epoch's slots to
        // `state.previous_slots` and deleting previous epoch's slots).
        let validators = if policy::epoch_at(self.block_number()) == policy::epoch_at(block_number)
            && !policy::is_election_block_at(self.block_number())
        {
            self.current_validators()?
        } else if (policy::epoch_at(self.block_number()) == policy::epoch_at(block_number)
            && policy::is_election_block_at(self.block_number()))
            || (policy::epoch_at(self.block_number()) == policy::epoch_at(block_number) + 1
                && !policy::is_election_block_at(self.block_number()))
        {
            self.previous_validators()?
        } else {
            self.get_block_at(
                policy::election_block_before(block_number),
                true,
                txn_option,
            )?
            .validators()?
        };

        // Finally get the correct validator.
        let validator = validators.get_validator(slot_number).clone();

        Some((validator, slot_number))
    }

    /// Calculate the slot owner number at a given block and view number.
    /// In combination with the active Validators, this can be used to retrieve the validator info.
    fn get_slot_owner_number_at(
        &self,
        block_number: u32,
        view_number: u32,
        disabled_slots: BitSet,
        txn_option: Option<&Transaction>,
    ) -> u16 {
        // Get the last block's seed.
        let seed = self
            .get_block_at(block_number - 1, false, txn_option)
            .expect("Can't find previous block!")
            .seed()
            .clone();

        // RNG for slot selection
        let mut rng = seed.rng(VrfUseCase::SlotSelection, view_number);

        // Check if all slots are disabled. In this case, we will accept any slot, since we want the
        // chain to progress.
        if disabled_slots.len() == policy::SLOTS as usize {
            // Sample a random slot number.
            return rng.next_u64_max(policy::SLOTS as u64) as u16;
        }

        // Sample a random index. See that we only consider the non-disabled slots here.
        let mut r =
            rng.next_u64_max((policy::SLOTS as usize - disabled_slots.len()) as u64) as usize;

        // Now we just iterate over all the slots until we find the r-th non-disabled slot.
        let mut slot_number = 0;
        for is_disabled in disabled_slots.iter_bits() {
            if is_disabled {
                slot_number += 1;
                continue;
            }

            if r == 0 {
                return slot_number;
            } else {
                slot_number += 1;
                r -= 1;
            }
        }

        unreachable!()
    }
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
}
