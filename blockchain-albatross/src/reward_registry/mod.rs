use std::borrow::Cow;
use std::io;

use failure::Fail;

use beserial::{Deserialize, Serialize};
use collections::bitset::BitSet;
use database::{AsDatabaseBytes, FromDatabaseValue};

pub use reward_pot::RewardPot;
pub use slash_registry::SlashRegistry;

mod reward_pot;
mod slash_registry;

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
