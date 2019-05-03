use beserial::{Deserialize, Serialize};

use nimiq_keys::{Signature, PublicKey};
use hash::{Hash, Blake2bHash, HashOutput, SerializeContent};
use std::io;
use beserial::WriteBytesExt;
use beserial::ReadBytesExt;
use beserial::SerializingError;
use crate::AggregateProof;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PbftJustification {
    prepare: AggregateProof,
    commit: AggregateProof
}

impl PbftJustification {
    pub fn verify(&self, hash: Blake2bHash) -> bool {
        unimplemented!()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PbftProof {
    pub hash: Blake2bHash,
    pub signature: Signature,
    pub public_key: PublicKey
}

impl SerializeContent for PbftProof {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { Ok(self.serialize(writer)?) }
}

impl Hash for PbftProof { }

impl PbftProof {
    pub fn verify(&self) {
        unimplemented!()
    }
}

