use beserial::{Deserialize, Serialize};

use nimiq_keys::{Signature, PublicKey};
use hash::{Hash, Blake2bHash, HashOutput, SerializeContent};
use std::io;
use beserial::WriteBytesExt;
use beserial::ReadBytesExt;
use beserial::SerializingError;
use super::signed;

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize, SerializeContent)]
pub struct ViewChange {
    pub block_number: u32,
    pub new_view_number: u16,
}

impl signed::Message for ViewChange {
    const PREFIX: u8 = signed::PREFIX_VIEW_CHANGE;
}

pub type SignedViewChange = signed::SignedMessage<ViewChange>;
