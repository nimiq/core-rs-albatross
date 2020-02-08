use std::fmt;

use beserial::{Deserialize, Serialize};
use hash::SerializeContent;
use hash_derive::SerializeContent;
use vrf::VrfSeed;

use super::signed;

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize, SerializeContent, Hash)]
pub struct ViewChange {
    /// The hash of the previous block.
    /// This is needed to distinguish view changes on different branches.
    /// We choose the seed so that the view change applies to all branches of a malicious fork,
    /// but not to branching because of view changes.
    pub prev_seed: VrfSeed,

    /// The number of the block for which the view change is constructed (i.e. the block number
    /// the validator is at + 1, since it's for the next block)
    pub block_number: u32,

    /// The view number after the view_change (i.e. the current view number + 1, except if the view
    /// change is for the first micro block of an epoch)
    pub new_view_number: u32,
}

impl signed::Message for ViewChange {
    const PREFIX: u8 = signed::PREFIX_VIEW_CHANGE;
}

pub type SignedViewChange = signed::SignedMessage<ViewChange>;
pub type ViewChangeProof = signed::AggregateProof<ViewChange>;
pub type ViewChangeProofBuilder = signed::AggregateProofBuilder<ViewChange>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewChanges {
    pub block_number: u32,
    /// The first view number that was changed
    pub first_view_number: u32,
    /// The last view number - i.e. the first one that wasn't changed
    pub last_view_number: u32,
}

impl ViewChanges {
    pub fn new(block_number: u32, first_view_number: u32, last_view_number: u32) -> Option<ViewChanges> {
        if first_view_number < last_view_number {
            Some(ViewChanges {
                block_number,
                first_view_number,
                last_view_number,
            })
        } else {
            None
        }
    }
}

impl fmt::Display for ViewChange {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "#{}.{} ({})", self.block_number, self.new_view_number, self.prev_seed)
    }
}
