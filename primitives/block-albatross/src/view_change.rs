use beserial::{Deserialize, Serialize};

use nimiq_keys::{Signature, PublicKey};
use hash::{Hash, Blake2bHash, HashOutput, SerializeContent};
use std::io;
use beserial::WriteBytesExt;
use beserial::ReadBytesExt;
use beserial::SerializingError;

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub struct ViewChangeInternals {
    pub block_number: u32,
    pub new_view_number: u16,
}

impl SerializeContent for ViewChangeInternals {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewChange {
    pub internals: ViewChangeInternals,
    pub signature: Signature,
    pub public_key: PublicKey,
}

impl ViewChange {
    pub fn verify(&self) -> bool {
        self.public_key.verify(&self.signature, &self.internals.serialize_to_vec()[..])
    }
}
