use beserial::{Deserialize, Serialize};

use hash::SerializeContent;
use super::signed;

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize, SerializeContent, Hash)]
pub struct ViewChange {
    pub block_number: u64,
    pub new_view_number: u32,
}

impl signed::Message for ViewChange {
    const PREFIX: u8 = signed::PREFIX_VIEW_CHANGE;
}

pub type SignedViewChange = signed::SignedMessage<ViewChange>;
pub type ViewChangeProof = signed::AggregateProof<ViewChange>;
