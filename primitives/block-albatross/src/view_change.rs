use beserial::{Deserialize, Serialize};

use hash::SerializeContent;
use super::signed;

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize, SerializeContent, Hash)]
pub struct ViewChange {
    pub block_number: u32,
    pub new_view_number: u32,
}

impl signed::Message for ViewChange {
    const PREFIX: u8 = signed::PREFIX_VIEW_CHANGE;
}

pub type SignedViewChange = signed::SignedMessage<ViewChange>;
pub type ViewChangeProof = signed::AggregateProof<ViewChange>;
pub type ViewChangeProofBuilder = signed::AggregateProofBuilder<ViewChange>;

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
