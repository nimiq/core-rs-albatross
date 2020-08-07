use crate::chain_info::ChainInfo;
use crate::chain_store::ChainStore;
use beserial::{Deserialize, Serialize};
use block::{Block, MicroBlock};
use collections::bitset::BitSet;
use database::Transaction;
use failure::Fail;
use primitives::policy;
use primitives::slot::{Slot, SlotIndex, Slots};
use vrf::{Rng, VrfUseCase};

#[derive(Debug, Fail)]
pub enum SlashPushError {
    #[fail(display = "Redundant fork proofs in block")]
    DuplicateForkProof,
    #[fail(display = "Block contains fork proof targeting a slot that was already slashed")]
    SlotAlreadySlashed,
    #[fail(display = "Fork proof is from a wrong epoch")]
    InvalidEpochTarget,
    #[fail(display = "Fork proof infos don't match fork proofs")]
    InvalidForkProofInfos,
    #[fail(display = "Fork proof infos cannot be fetched (predecessor does not exist)")]
    InvalidForkProofPredecessor,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct SlashedSet {
    pub view_change_epoch_state: BitSet,
    pub fork_proof_epoch_state: BitSet,
    pub prev_epoch_state: BitSet,
}

impl SlashedSet {
    /// Returns the full slashed set for the current epoch.
    pub fn current_epoch(&self) -> BitSet {
        &self.view_change_epoch_state | &self.fork_proof_epoch_state
    }

    /// Calculates the slashed set that is used to determine the block producers of
    /// the block at `next_block_number` if this is the slashed set after applying the previous
    /// block.
    ///
    /// That means that this function will return an empty current slashed set for the
    /// first block in an epoch.
    pub fn next_slashed_set(&self, next_block_number: u32) -> Self {
        // If `prev_info` is from the previous epoch, we switched epochs.
        if policy::is_macro_block_at(next_block_number - 1) {
            SlashedSet {
                view_change_epoch_state: Default::default(),
                fork_proof_epoch_state: Default::default(),
                prev_epoch_state: &self.fork_proof_epoch_state | &self.view_change_epoch_state,
            }
        } else {
            self.clone()
        }
    }
}

/// Get slot and slot number for a given block and view number.
pub fn get_slot_at(
    block_number: u32,
    view_number: u32,
    prev_chain_info: &ChainInfo,
    slots: &Slots,
) -> (Slot, u16) {
    // We need to have an up-to-date slashed set to be able to calculate the slots.
    assert_eq!(prev_chain_info.head.block_number(), block_number - 1);

    let slot_number = get_slot_number_at(block_number, view_number, prev_chain_info);
    let slot = slots
        .get(SlotIndex::Slot(slot_number))
        .unwrap_or_else(|| panic!("Expected slot {} to exist", slot_number));
    (slot, slot_number)
}

/// Calculate the slot number at a given block and view number.
/// In combination with the active `Slots`, this can be used to retrieve the validator info.
pub(crate) fn get_slot_number_at(
    block_number: u32,
    view_number: u32,
    prev_chain_info: &ChainInfo,
) -> u16 {
    // We need to have an up-to-date slashed set to be able to calculate the slots.
    assert_eq!(prev_chain_info.head.block_number(), block_number - 1);

    // TODO: Refactor:
    // * Use HashRng
    // * Instead of sampling from the honest "validators" (which are slots really),
    //   create a list of slot numbers that are still enabled. Then sample from that and
    //   you'll have the proper slot_number instead of a "honest slot index".

    // Get slashed set for epoch ignoring fork proofs.
    let slashed_set = prev_chain_info.next_slashed_set().view_change_epoch_state;

    // RNG for slot selection
    let mut rng = prev_chain_info
        .head
        .seed()
        .rng(VrfUseCase::SlotSelection, view_number);

    let slot_number = loop {
        let slot_number = rng.next_u64_max(policy::SLOTS as u64) as u16;

        // Sample until we find a slot that is not slashed
        if !slashed_set.contains(slot_number as usize) {
            break slot_number;
        }
    };

    slot_number
}

/// Mark fork proofs in `SlashedSet`.
fn slash_fork_proofs(
    block: &MicroBlock,
    fork_proof_infos: &ForkProofInfos,
    slashed_set: &mut SlashedSet,
) -> Result<(), SlashPushError> {
    let block_epoch = policy::epoch_at(block.header.block_number);

    let mut fork_proof_epoch_diff = BitSet::new();
    let mut fork_proof_prev_epoch_diff = BitSet::new();

    // Mark from fork proofs.
    let fork_proofs = &block.body.as_ref().unwrap().fork_proofs;
    if fork_proofs.len() != fork_proof_infos.predecessor_infos.len() {
        return Err(SlashPushError::InvalidForkProofInfos);
    }
    for (fork_proof, prev_info) in fork_proofs
        .iter()
        .zip(fork_proof_infos.predecessor_infos.iter())
    {
        let block_number = fork_proof.header1.block_number;
        let view_number = fork_proof.header1.view_number;
        if fork_proof.header1.parent_hash != prev_info.head.hash() {
            return Err(SlashPushError::InvalidForkProofInfos);
        }
        let slot_number = get_slot_number_at(block_number, view_number, &prev_info);

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

        // Detect duplicate fork proof slashes
        if !(&slashed_set.prev_epoch_state & &fork_proof_prev_epoch_diff).is_empty()
            || !(&slashed_set.fork_proof_epoch_state & &fork_proof_epoch_diff).is_empty()
        {
            return Err(SlashPushError::SlotAlreadySlashed);
        }
    }

    slashed_set.fork_proof_epoch_state |= fork_proof_epoch_diff;
    slashed_set.prev_epoch_state |= fork_proof_prev_epoch_diff;

    Ok(())
}

/// Mark view change slashes in `SlashedSet`.
fn slash_view_changes(
    block: &MicroBlock,
    prev_chain_info: &ChainInfo,
    slashed_set: &mut SlashedSet,
) {
    let prev_view_number = prev_chain_info.head.view_number();
    let view_number = block.header.view_number;
    let block_number = block.header.block_number;

    // Mark from view changes, ignoring duplicates.
    for view in prev_view_number..view_number {
        let slot_number = get_slot_number_at(block_number, view, prev_chain_info);
        slashed_set
            .view_change_epoch_state
            .insert(slot_number as usize);
    }
}

/// Given the previous block's `ChainInfo` and the current block,
/// this function constructs the new slashed set.
/// If a fork proof slashes a slot that has already been slashed,
/// the function returns a `SlashPushError`.
pub fn apply_slashes(
    block: &Block,
    prev_chain_info: &ChainInfo,
    fork_proof_infos: &ForkProofInfos,
) -> Result<SlashedSet, SlashPushError> {
    // We need to have an up-to-date slashed set.
    assert_eq!(&prev_chain_info.head.hash(), block.parent_hash());

    let mut slashed_set = prev_chain_info.next_slashed_set();

    // TODO: Do we apply slashes in macro blocks?
    if let Block::Micro(ref micro_block) = block {
        slash_fork_proofs(micro_block, fork_proof_infos, &mut slashed_set)?;
        slash_view_changes(micro_block, prev_chain_info, &mut slashed_set);
    }

    Ok(slashed_set)
}

/// A struct containing additional information required to process fork proofs.
pub struct ForkProofInfos {
    predecessor_infos: Vec<ChainInfo>,
}

impl ForkProofInfos {
    pub fn empty() -> Self {
        ForkProofInfos {
            predecessor_infos: vec![],
        }
    }

    pub fn fetch(
        block: &Block,
        chain_store: &ChainStore,
        txn: Option<&Transaction>,
    ) -> Result<Self, SlashPushError> {
        let mut infos = vec![];
        if let Block::Micro(micro_block) = block {
            let fork_proofs = &micro_block.body.as_ref().unwrap().fork_proofs;
            for fork_proof in fork_proofs {
                let prev_info = chain_store
                    .get_chain_info(&fork_proof.header1.parent_hash, false, txn)
                    .ok_or(SlashPushError::InvalidForkProofPredecessor)?;
                infos.push(prev_info);
            }
        }
        Ok(ForkProofInfos {
            predecessor_infos: infos,
        })
    }
}
