mod reward_pot;

use std::borrow::Cow;
use std::io;
use std::sync::Arc;

use failure::Fail;

use beserial::{Deserialize, Serialize};
use block::{Block, MacroBlock, MicroBlock};
use collections::bitset::BitSet;
use database::cursor::{ReadCursor, WriteCursor};
use database::{
    AsDatabaseBytes, Database, DatabaseFlags, Environment, FromDatabaseValue, ReadTransaction,
    Transaction, WriteTransaction,
};
use primitives::coin::Coin;
use primitives::policy;
use primitives::slot::{Slot, SlotIndex, Slots};
use transaction::Transaction as BlockchainTransaction;
use vrf::VrfUseCase;

use crate::chain_store::ChainStore;
use crate::reward_registry::reward_pot::RewardPot;
use vrf::rng::Rng;

pub struct SlashRegistry {
    env: Environment,
    chain_store: Arc<ChainStore>,
    slash_registry_db: Database,
    reward_pot: RewardPot,
}

// TODO: Better error messages
#[derive(Debug, Fail)]
pub enum SlashPushError {
    #[fail(display = "Redundant fork proofs in block")]
    DuplicateForkProof,
    #[fail(display = "Block contains fork proof targeting a slot that was already slashed")]
    SlotAlreadySlashed,
    #[fail(display = "Block slashes slots in wrong epoch")]
    InvalidEpochTarget,
    #[fail(display = "Got block with unexpected block number")]
    UnexpectedBlock,
}

#[derive(Debug, Fail)]
pub enum EpochStateError {
    #[fail(display = "Block precedes requested epoch")]
    BlockPrecedesEpoch,
    #[fail(display = "Requested epoch too old to be tracked at block number")]
    HistoricEpoch,
}

#[derive(Clone, Copy, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum SlashedSetSelector {
    ViewChanges,
    ForkProofs,
    All,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BlockDescriptor {
    view_change_epoch_state: BitSet,
    fork_proof_epoch_state: BitSet,
    prev_epoch_state: BitSet,
}

// TODO: Pass in active validator set + seed through parameters
//      or always load from chain store?
impl SlashRegistry {
    const SLASH_REGISTRY_DB_NAME: &'static str = "SlashRegistry";

    pub fn new(env: Environment, chain_store: Arc<ChainStore>) -> Self {
        let slash_registry_db = env.open_database_with_flags(
            SlashRegistry::SLASH_REGISTRY_DB_NAME.to_string(),
            DatabaseFlags::UINT_KEYS,
        );
        let reward_pot = RewardPot::new(env.clone());

        Self {
            env,
            chain_store,
            slash_registry_db,
            reward_pot,
        }
    }

    #[inline]
    pub fn current_reward_pot(&self) -> Coin {
        self.reward_pot.current_reward_pot()
    }

    #[inline]
    pub fn previous_reward_pot(&self) -> Coin {
        self.reward_pot.previous_reward_pot()
    }

    /// Register slashes of block
    ///  * `block` - Block to commit
    ///  * `seed`- Seed of previous block
    ///  * `staking_contract` - Contract used to check minimum stakes
    #[inline]
    pub fn commit_block(
        &self,
        txn: &mut WriteTransaction,
        block: &Block,
        prev_view_number: u32,
    ) -> Result<(), SlashPushError> {
        match block {
            Block::Macro(ref macro_block) => {
                self.reward_pot.commit_macro_block(macro_block, txn);
                self.commit_macro_block(txn, macro_block, prev_view_number)?;
                self.gc(txn, policy::epoch_at(macro_block.header.block_number));
                Ok(())
            }
            Block::Micro(ref micro_block) => {
                self.reward_pot.commit_micro_block(micro_block, txn);
                self.commit_micro_block(txn, micro_block, prev_view_number)
            }
        }
    }

    pub fn commit_epoch(
        &self,
        txn: &mut WriteTransaction,
        block_number: u32,
        transactions: &[BlockchainTransaction],
        view_change_slashed_slots: &BitSet,
    ) -> Result<(), SlashPushError> {
        self.reward_pot
            .commit_epoch(block_number, transactions, txn);

        // Just put the whole epochs slashed set at the macro blocks position.
        // We don't have slash info for the current epoch though.
        let descriptor = BlockDescriptor {
            view_change_epoch_state: BitSet::new(),
            fork_proof_epoch_state: BitSet::new(),
            prev_epoch_state: view_change_slashed_slots.clone(),
        };

        // Put descriptor into database.
        txn.put(&self.slash_registry_db, &block_number, &descriptor);
        self.gc(txn, policy::epoch_at(block_number));

        Ok(())
    }

    fn get_epoch_state(&self, txn: &mut WriteTransaction, block_number: u32) -> BlockDescriptor {
        let block_epoch = policy::epoch_at(block_number);

        // Lookup slash state.
        let mut cursor = txn.cursor(&self.slash_registry_db);

        // Move cursor to first entry with a block number >= ours (or end of the database).
        let _: Option<(u32, BlockDescriptor)> = cursor.seek_range_key(&block_number);

        // Then move cursor back by one.
        let last_change: Option<(u32, BlockDescriptor)> = cursor.prev();

        let prev_epoch_state: BitSet;
        let view_change_epoch_state: BitSet;
        let fork_proof_epoch_state: BitSet;
        if let Some((change_block_number, change)) = last_change {
            if change_block_number >= policy::first_block_of(block_epoch) {
                // last_change was in current epoch
                prev_epoch_state = change.prev_epoch_state;
                view_change_epoch_state = change.view_change_epoch_state;
                fork_proof_epoch_state = change.fork_proof_epoch_state;
            } else if block_epoch > 0
                && change_block_number >= policy::first_block_of(block_epoch - 1)
            {
                // last_change was in previous epoch
                // mingle slashes together
                prev_epoch_state = change.view_change_epoch_state | change.fork_proof_epoch_state;
                view_change_epoch_state = BitSet::new();
                fork_proof_epoch_state = BitSet::new();
            } else {
                // no change in the last two epochs
                prev_epoch_state = BitSet::new();
                view_change_epoch_state = BitSet::new();
                fork_proof_epoch_state = BitSet::new();
            }
        } else {
            // no change at all
            prev_epoch_state = BitSet::new();
            view_change_epoch_state = BitSet::new();
            fork_proof_epoch_state = BitSet::new();
        }

        BlockDescriptor {
            prev_epoch_state,
            view_change_epoch_state,
            fork_proof_epoch_state,
        }
    }

    fn slash_view_changes(
        &self,
        epoch_diff: &mut BitSet,
        txn: &mut WriteTransaction,
        block_number: u32,
        view_number: u32,
        prev_view_number: u32,
    ) {
        // Mark from view changes, ignoring duplicates.
        for view in prev_view_number..view_number {
            let slot_number = self
                .get_slot_number_at(block_number, view, Some(&txn))
                .unwrap();
            epoch_diff.insert(slot_number as usize);
        }
    }

    fn commit_macro_block(
        &self,
        txn: &mut WriteTransaction,
        block: &MacroBlock,
        prev_view_number: u32,
    ) -> Result<(), SlashPushError> {
        let mut epoch_diff = BitSet::new();

        let BlockDescriptor {
            fork_proof_epoch_state,
            prev_epoch_state,
            mut view_change_epoch_state,
        } = self.get_epoch_state(txn, block.header.block_number);

        self.slash_view_changes(
            &mut epoch_diff,
            txn,
            block.header.block_number,
            block.header.view_number,
            prev_view_number,
        );

        // Apply slashes.
        view_change_epoch_state |= epoch_diff;

        // Push block descriptor and remember slash hashes.
        let descriptor = BlockDescriptor {
            view_change_epoch_state,
            fork_proof_epoch_state,
            prev_epoch_state,
        };

        // Put descriptor into database.
        txn.put(
            &self.slash_registry_db,
            &block.header.block_number,
            &descriptor,
        );

        Ok(())
    }

    fn commit_micro_block(
        &self,
        txn: &mut WriteTransaction,
        block: &MicroBlock,
        prev_view_number: u32,
    ) -> Result<(), SlashPushError> {
        let block_epoch = policy::epoch_at(block.header.block_number);
        let mut view_change_epoch_diff = BitSet::new();
        let mut fork_proof_epoch_diff = BitSet::new();
        let mut fork_proof_prev_epoch_diff = BitSet::new();

        // Mark from fork proofs.
        let fork_proofs = &block.extrinsics.as_ref().unwrap().fork_proofs;
        for fork_proof in fork_proofs {
            let block_number = fork_proof.header1.block_number;
            let view_number = fork_proof.header1.view_number;
            let slot_number = self
                .get_slot_number_at(block_number, view_number, Some(&txn))
                .unwrap();

            let slash_epoch = policy::epoch_at(block_number);
            if block_epoch == slash_epoch {
                if fork_proof_epoch_diff.contains(slot_number as usize) {
                    return Err(SlashPushError::DuplicateForkProof);
                }
                fork_proof_epoch_diff.insert(slot_number as usize);
            } else if block_epoch == slash_epoch + 1 {
                if fork_proof_prev_epoch_diff.contains(slot_number as usize) {
                    return Err(SlashPushError::DuplicateForkProof);
                }
                fork_proof_prev_epoch_diff.insert(slot_number as usize);
            } else {
                return Err(SlashPushError::InvalidEpochTarget);
            }
        }

        let BlockDescriptor {
            mut fork_proof_epoch_state,
            mut prev_epoch_state,
            mut view_change_epoch_state,
        } = self.get_epoch_state(txn, block.header.block_number);

        // Detect duplicate fork proof slashes
        if !(&prev_epoch_state & &fork_proof_prev_epoch_diff).is_empty()
            || !(&fork_proof_epoch_state & &fork_proof_epoch_diff).is_empty()
        {
            return Err(SlashPushError::SlotAlreadySlashed);
        }

        self.slash_view_changes(
            &mut view_change_epoch_diff,
            txn,
            block.header.block_number,
            block.header.view_number,
            prev_view_number,
        );

        // Apply slashes.
        prev_epoch_state |= fork_proof_prev_epoch_diff;
        fork_proof_epoch_state |= fork_proof_epoch_diff;
        view_change_epoch_state |= view_change_epoch_diff;

        // Push block descriptor and remember slash hashes.
        let descriptor = BlockDescriptor {
            view_change_epoch_state,
            fork_proof_epoch_state,
            prev_epoch_state,
        };

        // Put descriptor into database.
        txn.put(
            &self.slash_registry_db,
            &block.header.block_number,
            &descriptor,
        );

        Ok(())
    }

    fn gc(&self, txn: &mut WriteTransaction, current_epoch: u32) {
        let cutoff = policy::first_block_of_registry(current_epoch);
        if cutoff == 0u32 {
            // The first possible block is part of the registry.
            // We can't delete any descriptors.
            return;
        }

        let mut cursor = txn.write_cursor(&self.slash_registry_db);
        let mut pos: Option<(u32, BlockDescriptor)> = cursor.first();

        while let Some((block_number, _)) = pos {
            if block_number >= cutoff {
                return;
            }
            cursor.remove();
            pos = cursor.next();
        }
    }

    #[inline]
    pub fn revert_block(
        &self,
        txn: &mut WriteTransaction,
        block: &Block,
    ) -> Result<(), SlashPushError> {
        if let Block::Micro(ref block) = block {
            self.reward_pot.revert_micro_block(block, txn);
            self.revert_micro_block(txn, block)
        } else {
            unreachable!()
        }
    }

    fn revert_micro_block(
        &self,
        txn: &mut WriteTransaction,
        block: &MicroBlock,
    ) -> Result<(), SlashPushError> {
        txn.remove(&self.slash_registry_db, &block.header.block_number);
        Ok(())
    }

    /// Get slot and slot number for a given block and view number
    pub fn get_slot_at(
        &self,
        block_number: u32,
        view_number: u32,
        slots: &Slots,
        txn_option: Option<&Transaction>,
    ) -> Option<(Slot, u16)> {
        let slot_number = self.get_slot_number_at(block_number, view_number, txn_option)?;
        let slot = slots
            .get(SlotIndex::Slot(slot_number))
            .unwrap_or_else(|| panic!("Expected slot {} to exist", slot_number));
        Some((slot, slot_number))
    }

    /// TODO: Return an error for this, so we don't have to write the same error message for `expect`s all over the place.
    pub(crate) fn get_slot_number_at(
        &self,
        block_number: u32,
        view_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<u16> {
        // Get context
        let prev_block = self
            .chain_store
            .get_block_at(block_number - 1, false, txn_option)?;

        // TODO: Refactor:
        // * Use HashRng
        // * Instead of sampling from the honest "validators" (which are slots really),
        //   create a list of slot numbers that are still enabled. Then sample from that and
        //   you'll have the proper slot_number instead of a "honest slot index".

        // Get slashed set for epoch ignoring fork proofs.
        let slashed_set = self
            .slashed_set_at(
                policy::epoch_at(block_number),
                block_number,
                SlashedSetSelector::ViewChanges,
                txn_option,
            )
            .ok()?;

        // RNG for slot selection
        let mut rng = prev_block
            .seed()
            .rng(VrfUseCase::SlotSelection, view_number);

        let slot_number = loop {
            let slot_number = rng.next_u64_max(policy::SLOTS as u64) as u16;

            // Sample until we find a slot that is not slashed
            if !slashed_set.contains(slot_number as usize) {
                break slot_number;
            }
        };

        Some(slot_number)
    }

    /// Get latest known slash set of epoch
    pub fn slashed_set(
        &self,
        epoch_number: u32,
        set_selector: SlashedSetSelector,
        txn_option: Option<&Transaction>,
    ) -> BitSet {
        self.slashed_set_at(
            epoch_number,
            policy::first_block_of(epoch_number + 2),
            set_selector,
            txn_option,
        )
        .unwrap()
    }

    fn select_slashed_set(descriptor: BlockDescriptor, selector: SlashedSetSelector) -> BitSet {
        match selector {
            SlashedSetSelector::ViewChanges => descriptor.view_change_epoch_state,
            SlashedSetSelector::ForkProofs => descriptor.fork_proof_epoch_state,
            SlashedSetSelector::All => {
                descriptor.view_change_epoch_state | descriptor.fork_proof_epoch_state
            }
        }
    }

    /// Get slash set of epoch at specific block number
    /// Returns slash set before applying block with that block_number (TODO Tests)
    /// This includes view change slashes, but excludes fork proof slashes!
    pub fn slashed_set_at(
        &self,
        epoch_number: u32,
        block_number: u32,
        set_selector: SlashedSetSelector,
        txn_option: Option<&Transaction>,
    ) -> Result<BitSet, EpochStateError> {
        let epoch_start = policy::first_block_of(policy::epoch_at(block_number));

        // Epoch cannot have slashes if in the future
        if block_number < epoch_start {
            return Err(EpochStateError::BlockPrecedesEpoch);
        }

        // Epoch slashes are only tracked for two epochs
        // First block of (epoch + 2) is fine because upper lookup bound is exclusive.
        if block_number > policy::first_block_of(epoch_number + 2) {
            return Err(EpochStateError::HistoricEpoch);
        }

        let read_txn;
        let txn = if let Some(txn) = txn_option {
            txn
        } else {
            read_txn = ReadTransaction::new(&self.env);
            &read_txn
        };

        // Lookup slash state.
        let mut cursor = txn.cursor(&self.slash_registry_db);
        // Move cursor to first entry with a block number >= ours (or end of the database).
        let _: Option<(u32, BlockDescriptor)> = cursor.seek_range_key(&block_number);
        // Then move cursor back by one.
        let last_change: Option<(u32, BlockDescriptor)> = cursor.prev();

        if let Some((change_block_number, change)) = last_change {
            if change_block_number >= epoch_start {
                Ok(SlashRegistry::select_slashed_set(change, set_selector))
            } else {
                Ok(BitSet::new())
            }
        } else {
            Ok(BitSet::new())
        }
    }
}

impl AsDatabaseBytes for BlockDescriptor {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        let v = Serialize::serialize_to_vec(&self);
        Cow::Owned(v)
    }
}

impl FromDatabaseValue for BlockDescriptor {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
