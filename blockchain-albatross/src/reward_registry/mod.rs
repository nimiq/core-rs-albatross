use std::borrow::Cow;
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::io;
use std::io::Write;
use std::ops::Bound::{Excluded, Included, Unbounded};
use std::sync::Arc;

use failure::Fail;

use account::StakingContract;
use beserial::{Deserialize, Serialize};
use block::{Block, MacroBlock, MicroBlock};
use bls::bls12_381::CompressedSignature as CompressedBlsSignature;
use collections::bitset::BitSet;
use database::{AsDatabaseBytes, Database, DatabaseFlags, Environment, FromDatabaseValue, ReadTransaction, WriteTransaction};
use hash::{Blake2bHasher, Hasher};
use primitives::coin::Coin;
use primitives::policy;
use primitives::validators::{Slot, Slots};

use crate::chain_store::ChainStore;
use crate::reward_registry::reward_pot::RewardPot;

mod reward_pot;

pub struct SlashRegistry<'env> {
    env: &'env Environment,
    chain_store: Arc<ChainStore<'env>>,
    slash_registry_db: Database<'env>,
    reward_pot: RewardPot<'env>,
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

// TODO Pass in active validator set + seed through parameters
//      or always load from chain store?
impl<'env> SlashRegistry<'env> {
    const SLASH_REGISTRY_DB_NAME: &'static str = "SlashRegistry";

    pub fn new(env: &'env Environment, chain_store: Arc<ChainStore<'env>>) -> Self {
        let slash_registry_db = env.open_database_with_flags(SlashRegistry::SLASH_REGISTRY_DB_NAME.to_string(), DatabaseFlags::UINT_KEYS);

        Self {
            env,
            chain_store,
            slash_registry_db,
            reward_pot: RewardPot::new(env),
        }
    }

    /// Register slashes of block
    ///  * `block` - Block to commit
    ///  * `seed`- Seed of previous block
    ///  * `staking_contract` - Contract used to check minimum stakes
    #[inline]
    pub fn commit_block(&self, txn: &mut WriteTransaction, block: &Block) -> Result<(), SlashPushError> {
        match block {
            // TODO: Run GC on macro block
            Block::Macro(_) => Ok(()),
            Block::Micro(ref micro_block) => self.commit_micro_block(txn, micro_block),
        }
    }

    pub fn commit_micro_block(&self, txn: &mut WriteTransaction, block: &MicroBlock) -> Result<(), SlashPushError> {
        let block_epoch = policy::epoch_at(block.header.block_number);
        let mut epoch_diff = BitSet::new();
        let mut prev_epoch_diff = BitSet::new();

        // Mark from fork proofs.
        let fork_proofs = &block.extrinsics.as_ref().unwrap().fork_proofs;
        for fork_proof in fork_proofs {
            let block_number = fork_proof.header1.block_number;
            let view_number = fork_proof.header1.view_number;
            let slot_owner = self.slot_owner(block_number, view_number);

            let slash_epoch = policy::epoch_at(block_number);
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

        // Lookup slash state.
        let mut cursor = txn.cursor(&self.slash_registry_db);
        // Move cursor to first entry with a block number >= ours (or end of the database).
        let _: Option<(u32, BlockDescriptor)> = cursor.seek_range_key(&block.header.block_number);
        // Then move cursor back by one.
        let last_change: Option<(u32, BlockDescriptor)> = cursor.prev();

        let mut prev_epoch_state: BitSet;
        let mut epoch_state: BitSet;
        if let Some((change_block_number, change)) = last_change {
            if change_block_number >= policy::first_block_of(block_epoch) {
                // last_change was in current epoch
                prev_epoch_state = change.prev_epoch_state;
                epoch_state = change.epoch_state;
            } else if block_epoch > 0 && change_block_number >= policy::first_block_of(block_epoch - 1) {
                // last_change was in previous epoch
                prev_epoch_state = change.epoch_state;
                epoch_state = BitSet::new();
            } else {
                // no change in the last two epochs
                prev_epoch_state = BitSet::new();
                epoch_state = BitSet::new();
            }
        } else {
            // no change at all
            prev_epoch_state = BitSet::new();
            epoch_state = BitSet::new();
        }

        drop(cursor);

        // Detect duplicate slashes
        if (&prev_epoch_state & &prev_epoch_diff).len() != 0
            || (&epoch_state & &epoch_diff).len() != 0 {
            return Err(SlashPushError::SlotAlreadySlashed);
        }

        // Mark from view changes, ignoring duplicates.
        for view in 0..block.header.view_number {
            let slot_owner = self.slot_owner(block.header.block_number, view);
            epoch_diff.insert(slot_owner.0 as usize);
        }

        // Apply slashes.
        prev_epoch_state |= prev_epoch_diff;
        epoch_state |= epoch_diff;

        // Push block descriptor and remember slash hashes.
        let descriptor = BlockDescriptor { epoch_state, prev_epoch_state };

        // Put descriptor into database.
        txn.put(&self.slash_registry_db, &block.header.block_number, &descriptor);

        Ok(())
    }

    fn safe_subtract(a: Coin, b: Coin) -> Coin {
        if a < b {
            Coin::ZERO
        } else {
            a - b
        }
    }

    fn gc(&self, txn: &mut WriteTransaction, current_epoch: u32) {
        let cutoff = if current_epoch > 2 {
            policy::first_block_of(current_epoch - 1)
        } else {
            0u32
        };

//        let mut cursor = txn.cursor(&self.slash_registry_db);
//        let mut pos = cursor.first();

        // TODO: Implementation requires changes in LMDB interface.
//        while let Some(block_number, _) = pos {
//            if block_number < cutoff {
//                cursor.
//            }
//
//            pos = cursor.next();
//        }
    }

    #[inline]
    pub fn revert_block(&self, txn: &mut WriteTransaction, block: &Block) -> Result<(), SlashPushError> {
        if let Block::Micro(ref block) = block {
            self.revert_micro_block(txn, block)
        } else {
            unreachable!()
        }
    }

    pub fn revert_micro_block(&self, txn: &mut WriteTransaction, block: &MicroBlock) -> Result<(), SlashPushError> {
        txn.remove(&self.slash_registry_db, &block.header.block_number);
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

    fn without_slashes(slots: &Slots, slashes: Option<BitSet>) -> Vec<&Slot> {
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

    pub fn slash_bitset(&self, epoch_number: u32) -> Option<BitSet> {
        let txn = ReadTransaction::new(self.env);

        // Lookup slash state.
        let mut cursor = txn.cursor(&self.slash_registry_db);
        // Move cursor to first entry with a block number >= ours (or end of the database).
        let _: Option<(u32, BlockDescriptor)> = cursor.seek_range_key(&policy::first_block_of(epoch_number + 2));
        // Then move cursor back by one.
        let last_change: Option<(u32, BlockDescriptor)> = cursor.prev();

        if let Some((change_block_number, change)) = last_change {
            if change_block_number >= policy::first_block_of(epoch_number + 1) {
                return Some(change.prev_epoch_state);
            } else if change_block_number >= policy::first_block_of(epoch_number) {
                return Some(change.epoch_state);
            }
        }
        None
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

        let txn = ReadTransaction::new(self.env);

        // Lookup slash state.
        let mut cursor = txn.cursor(&self.slash_registry_db);
        // Move cursor to first entry with a block number >= ours (or end of the database).
        let _: Option<(u32, BlockDescriptor)> = cursor.seek_range_key(&block_number);
        // Then move cursor back by one.
        let last_change: Option<(u32, BlockDescriptor)> = cursor.prev();

        let slashes = if let Some((change_block_number, change)) = last_change {
            if change_block_number >= epoch_start {
                Some(change.epoch_state)
            } else {
                None
            }
        } else { None };

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
