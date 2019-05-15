use std::borrow::Cow;
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::io;
use std::io::Write;
use std::ops::Bound::{Unbounded, Included, Excluded};
use std::sync::Arc;

use failure::Fail;

use account::StakingContract;
use beserial::{Deserialize, Serialize};
use block::{Block, MicroBlock, MacroBlock};
use bls::bls12_381::CompressedSignature as CompressedBlsSignature;
use collections::bitset::BitSet;
use database::{AsDatabaseBytes, Database, Environment, FromDatabaseValue, ReadTransaction, WriteTransaction};
use hash::{Blake2bHasher, Hasher};
use primitives::coin::Coin;
use primitives::policy;
use primitives::validators::{Slot, Slots};

use crate::chain_store::ChainStore;

// TODO Remove bounds
// TODO Remove diff_heights
mod reward_pot;

pub struct SlashRegistry<'env> {
    env: &'env Environment,
    chain_store: Arc<ChainStore<'env>>,
    bounds: TrackedRange,
    diff_heights: BTreeMap<u32, BlockDescriptor>,
    slash_registry_db: Database<'env>,
}

// TODO Better error messages
#[derive(Debug, Fail)]
pub enum SlashPushError {
    #[fail(display = "Redundant fork proofs in block")]
    DuplicateForkProof,
    #[fail(display = "Block contains fork proof targeting a slot that was already slashes")]
    SlotAlreadySlashed,
    #[fail(display = "Block slashes slots in wrong epoch")]
    InvalidEpochTarget,
    #[fail(display = "Got block with unexpected block number")]
    UnexpectedBlock,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BlockDescriptor {
    epoch_state: BitSet,
    prev_epoch_state: BitSet,
}

// Inclusive
#[derive(Debug, Clone, Deserialize, Serialize)]
struct TrackedRange(u32, u32);

// TODO Pass in active validator set + seed through parameters
//      or always load from chain store?
impl<'env> SlashRegistry<'env> {
    const SLASH_REGISTRY_DB_NAME: &'static str = "SlashRegistry";
    const BOUNDS_KEY: &'static str = "bounds";

    pub fn new(env: &'env Environment, chain_store: Arc<ChainStore<'env>>) -> Self {
        let slash_registry_db = env.open_database(SlashRegistry::SLASH_REGISTRY_DB_NAME.to_string());

        let txn = ReadTransaction::new(env);
        let bounds: TrackedRange = txn.get(&slash_registry_db, SlashRegistry::BOUNDS_KEY).unwrap_or(TrackedRange(0u32, 0u32));

        let mut diff_heights = BTreeMap::<u32, BlockDescriptor>::new();

        {
            let mut diffs = txn.cursor(&slash_registry_db);
            while let Some((height, descriptor)) = diffs.next::<u32, BlockDescriptor>() {
                diff_heights.insert(height, descriptor.clone());
            }
        }

        Self {
            env,
            chain_store,
            bounds,
            diff_heights,
            slash_registry_db
        }
    }

    /// Register slashes of block
    ///  * `block` - Block to commit
    ///  * `seed`- Seed of previous block
    ///  * `staking_contract` - Contract used to check minimum stakes
    #[inline]
    pub fn commit_block(&mut self, txn: &mut WriteTransaction, block: &Block) -> Result<(), SlashPushError> {
        match block {
            Block::Macro(_) => Ok(()),
            Block::Micro(ref micro_block) => self.commit_micro_block(txn, micro_block),
        }
    }

    pub fn commit_micro_block(&mut self, txn: &mut WriteTransaction, block: &MicroBlock) -> Result<(), SlashPushError> {
        if !policy::successive_micro_blocks(self.bounds.1, block.header.block_number) {
            return Err(SlashPushError::UnexpectedBlock);
        }

        let block_epoch = policy::epoch_at(block.header.block_number);
        let mut epoch_diff = BitSet::new();
        let mut prev_epoch_diff = BitSet::new();

        // Mark from fork proofs
        let fork_proofs = &block.extrinsics.as_ref().unwrap().fork_proofs;
        for fork_proof in fork_proofs {
            let block_number = fork_proof.header1.block_number;
            let view_number = fork_proof.header1.view_number;
            let prev_block = self.chain_store.get_block_at(block_number - 1).unwrap();
            let slot_owner = self.slot_owner(block_number, view_number);

            let slash_epoch = fork_proof.header1.block_number;
            if block_epoch == slash_epoch {
                if epoch_diff.contains(slot_owner.0 as usize) {
                    return Err(SlashPushError::DuplicateForkProof);
                }
                epoch_diff.insert(slot_owner.0 as usize);
            } else if block_epoch == slash_epoch + 1 {
                if prev_epoch_diff.contains(slot_owner.0 as usize) {
                    return Err(SlashPushError::DuplicateForkProof);
                }
                prev_epoch_diff.insert(slot_owner.0 as usize);
            } else {
                return Err(SlashPushError::InvalidEpochTarget);
            }
        }

        // Lookup slash state
        let mut lookup_range = self.diff_heights.range((
            Included(policy::first_block_of(policy::epoch_at(block.header.block_number))),
            Excluded(block.header.block_number),
        ));
        let last_change = lookup_range.next_back();

        let mut prev_epoch_state: BitSet;
        let mut epoch_state: BitSet;

        if let Some(ref change) = last_change {
            prev_epoch_state = change.1.prev_epoch_state.clone();
            epoch_state = change.1.epoch_state.clone();
        } else {
            // Lookup state from previous epüoch
            let mut prev_lookup_range = self.diff_heights.range((
                Included(policy::first_block_of(block_epoch - 1)),
                Excluded(policy::first_block_of(block_epoch)),
            ));
            let prev_last_change = prev_lookup_range.next_back();
            prev_epoch_state = prev_last_change.map(|entry| entry.1.epoch_state.clone()).unwrap_or(BitSet::new());
            epoch_state = BitSet::new();
        }

        // Detect duplicate slashes
        if (&prev_epoch_state & &prev_epoch_diff).len() != 0
            || (&epoch_state & &epoch_diff).len() != 0 {
            return Err(SlashPushError::SlotAlreadySlashed);
        }

        // Mark from view changes, ignoring duplicates
        for view in 0..block.header.view_number {
            let slot_owner = self.slot_owner(block.header.block_number, view);
            epoch_diff.insert(slot_owner.0 as usize);
        }

        // Apply slashes
        prev_epoch_state |= prev_epoch_diff;
        epoch_state |= epoch_diff;

        // Push block descriptor and remember slash hashes
        let descriptor = BlockDescriptor { epoch_state, prev_epoch_state };

        // Commit and remove old entries
        self.bounds.1 = block.header.block_number;
        self.gc(txn);
        txn.put(&self.slash_registry_db, &block.header.block_number, &descriptor);
        self.diff_heights.insert(block.header.block_number, descriptor);
        txn.put(&self.slash_registry_db, Self::BOUNDS_KEY, &self.bounds);

        Ok(())
    }

    fn safe_subtract(a: Coin, b: Coin) -> Coin {
        if a < b {
            Coin::ZERO
        } else {
            a - b
        }
    }

    fn gc(&mut self, txn: &mut WriteTransaction) {
        let current_epoch = policy::epoch_at(self.bounds.1);
        let cutoff = if current_epoch > 2 {
            policy::first_block_of(current_epoch - 2)
        } else {
            0u32
        };
        loop {
            let mut height = 0u32;
            {
                let mut lookup_range = self.diff_heights.range((
                    Unbounded,
                    Excluded(cutoff),
                ));
                let to_remove = lookup_range.next();
                if to_remove.is_none() {
                    break;
                }
                height = *to_remove.unwrap().0;
            }
            self.diff_heights.remove(&height);
            txn.remove(&self.slash_registry_db, &height);
        }
        self.bounds.0 = cutoff;
        txn.put(&self.slash_registry_db, Self::BOUNDS_KEY, &self.bounds);
    }

    #[inline]
    pub fn revert_block(&mut self, txn: &mut WriteTransaction, block: &Block) -> Result<(), SlashPushError> {
        if let Block::Micro(ref block) = block {
            self.revert_micro_block(txn, block)
        } else {
            Ok(())
        }
    }

    pub fn revert_micro_block(&mut self, txn: &mut WriteTransaction, block: &MicroBlock) -> Result<(), SlashPushError> {
        if block.header.block_number != self.bounds.1 {
            return Err(SlashPushError::UnexpectedBlock);
        }

        self.diff_heights.remove(&block.header.block_number);
        txn.remove(&self.slash_registry_db, &block.header.block_number);
        self.bounds.1 -= 1;
        txn.put(&self.slash_registry_db, Self::BOUNDS_KEY, &self.bounds);

        Ok(())
    }

    // Get slot owner at block and view number
    pub fn slot_owner(&self, block_number: u32, view_number: u32) -> (u32, Slot) {
        let epoch_number = policy::epoch_at(block_number);

        // Get context
        let macro_block = self.chain_store
            .get_block_at(policy::macro_block_of(epoch_number - 1))
            .expect("Failed to determine slot owner - preceding macro block not found")
            .unwrap_macro();
        let prev_block = self.chain_store
            .get_block_at(block_number - 1)
            .expect("Failed to determine slot owner - preceding block not found");

        // Get slots of epoch
        let slots: Slots = macro_block.try_into().unwrap();
        let honest_validators = self.enabled_slots(block_number, &slots);

        // Hash seed and index
        let mut hash_state = Blake2bHasher::new();
        prev_block.seed().serialize(&mut hash_state);
        hash_state.write(&view_number.to_be_bytes());
        let hash = hash_state.finish();

        // Get number from first 8 bytes
        let mut num_bytes = [0u8; 8];
        num_bytes.copy_from_slice(&hash.as_bytes()[..8]);
        let num = u64::from_be_bytes(num_bytes);

        let index = num % honest_validators.len() as u64;
        (index as u32, honest_validators[index as usize].clone())
    }

    fn without_slashes<'a>(slots: &'a Slots, slashes: Option<&BitSet>) -> Vec<&'a Slot> {
        let mut idx = 0usize;
        let mut vec = Vec::<&Slot>::with_capacity(slots.len());

        // Iterate over slashed slots bit set
        if let Some(slashes) = slashes {
            for slashed in slashes.iter_bits() {
                if !slashed {
                    vec.push(&slots.get(idx))
                }
                idx += 1;
            }
        }

        // Copy rest if BitSet smaller than list of validators
        for i in idx..slots.len() {
            vec.push(slots.get(i));
        }

        vec
    }

    pub fn slash_bitset(&self, epoch_number: u32) -> Option<&BitSet> {
        let mut lookup_range = self.diff_heights.range((
            Included(policy::first_block_of(epoch_number)),
            Excluded(policy::first_block_of(epoch_number + 1)),
        ));
        let last_change = lookup_range.next_back();
        if let Some(ref entry) = last_change {
            Some(&entry.1.epoch_state)
        } else {
            let prev_lookup_range = self.diff_heights.range((
                Included(policy::first_block_of(epoch_number + 1)),
                Excluded(policy::first_block_of(epoch_number + 2)),
            ));
            lookup_range.next_back().as_ref().map(|entry| &entry.1.prev_epoch_state)
        }
    }

    // Reward eligible slots at epoch (can change in next epoch)
    pub fn reward_eligible(&self, epoch_number: u32) -> Vec<Slot> {
        let macro_block_stored = self.chain_store.get_block_at(policy::macro_block_of(epoch_number - 1)).unwrap();
        let macro_block = macro_block_stored.unwrap_macro();
        let slots: Slots = macro_block.try_into().unwrap();
        let slash_bitset = self.slash_bitset(epoch_number);
        Self::without_slashes(&slots, slash_bitset).iter().map(|slot| (*slot).clone()).collect()
    }

    // Get enabled slots up to block number
    fn enabled_slots<'a>(&self, block_number: u32, slots: &'a Slots) -> Vec<&'a Slot> {
        // Get last change of slots (diff.block_number <= block_number)
        let epoch_start = policy::first_block_of(policy::epoch_at(block_number));
        let mut lookup_range = self.diff_heights.range((Included(epoch_start), Excluded(block_number)));
        let slashes = lookup_range.next_back().as_ref().map(|entry| &entry.1.epoch_state);

        Self::without_slashes(slots, slashes)
    }
}

impl AsDatabaseBytes for BlockDescriptor {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        let v = Serialize::serialize_to_vec(&self);
        Cow::Owned(v)
    }
}

impl FromDatabaseValue for BlockDescriptor {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}

impl AsDatabaseBytes for TrackedRange {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        let v = Serialize::serialize_to_vec(&self);
        Cow::Owned(v)
    }
}

impl FromDatabaseValue for TrackedRange {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
