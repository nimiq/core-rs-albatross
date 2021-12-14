use nimiq_block::{Block, BlockType, MacroBlock};
use nimiq_collections::BitSet;
use nimiq_database::Transaction;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy;
use nimiq_primitives::slots::{Validator, Validators};
use nimiq_vrf::{Rng, VrfSeed, VrfUseCase};

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
    /// block number and view number with an optional previous seed.
    /// If the seed is none the previous block is retrieved to get the seed instead.
    fn get_slot_owner_with_seed(
        &self,
        block_number: u32,
        view_number: u32,
        prev_seed: Option<VrfSeed>,
        txn_option: Option<&Transaction>,
    ) -> Option<(Validator, u16)> {
        // The genesis block doesn't technically have a view slot list.
        if block_number == 0 {
            return None;
        }

        // Get the disabled slots for the current batch.
        let disabled_slots = self
            .get_block_at(policy::macro_block_before(block_number), true, txn_option)?
            .unwrap_macro()
            .body
            .unwrap()
            .disabled_set;

        // Get the slot number for the current block.
        let slot_number = self.get_slot_owner_number_with_seed(
            block_number,
            view_number,
            disabled_slots,
            prev_seed,
            txn_option,
        )?;

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
        let validator = validators.get_validator_by_slot_number(slot_number).clone();

        Some((validator, slot_number))
    }

    /// Calculates the slot owner (represented as the validator plus the slot number) at a given
    /// block number and view number.
    #[inline]
    fn get_slot_owner_at(
        &self,
        block_number: u32,
        view_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<(Validator, u16)> {
        self.get_slot_owner_with_seed(block_number, view_number, None, txn_option)
    }

    /// Calculates the slot owner number at a given block number and view number with an optional previous seed.
    /// If the seed is none the previous block is retrieved to get the seed instead.
    /// In combination with the active Validators, this can be used to retrieve the validator info.
    fn get_slot_owner_number_with_seed(
        &self,
        block_number: u32,
        view_number: u32,
        disabled_slots: BitSet,
        seed: Option<VrfSeed>,
        txn_option: Option<&Transaction>,
    ) -> Option<u16> {
        let seed = seed.or_else(|| {
            Some(
                self.get_block_at(block_number - 1, false, txn_option)?
                    .seed()
                    .clone(),
            )
        })?;
        // RNG for slot selection
        let mut rng = seed.rng(VrfUseCase::ViewSlotSelection);

        // Create a list of viable slots.
        let mut slots = vec![];

        if disabled_slots.len() == policy::SLOTS as usize {
            // If all slots are disabled, we will accept any slot, since we want the
            // chain to progress.
            slots = (0..policy::SLOTS).collect();
        } else {
            // Otherwise, we will only accept slots that are not disabled.
            for i in 0..policy::SLOTS {
                if !disabled_slots.contains(i as usize) {
                    slots.push(i);
                }
            }
        }

        // Shuffle the slots vector using the Fisher–Yates shuffle.
        for i in (1..slots.len()).rev() {
            let r = rng.next_u64_max((i + 1) as u64) as usize;
            slots.swap(r, i);
        }

        // Now simply take the view number modulo the number of viable slots and that will give us
        // the chosen slot.
        Some(slots[view_number as usize % slots.len()])
    }

    /// Calculate the slot owner number at a given block and view number.
    /// In combination with the active Validators, this can be used to retrieve the validator info.
    #[inline]
    fn get_slot_owner_number_at(
        &self,
        block_number: u32,
        view_number: u32,
        disabled_slots: BitSet,
        txn_option: Option<&Transaction>,
    ) -> Option<u16> {
        self.get_slot_owner_number_with_seed(
            block_number,
            view_number,
            disabled_slots,
            None,
            txn_option,
        )
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
}
