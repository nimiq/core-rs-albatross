use beserial::{Deserialize, Serialize};

use nimiq_keys::{Signature, PublicKey};
use hash::{Hash, Blake2bHash, HashOutput, SerializeContent};
use std::io;
use pairing::bls12_381::Bls12;
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
        if !self.prepare.keys.verify(hash.as_bytes(), &self.prepare.signatures) {
            return false;
        }
        if !self.commit.keys.verify(hash.as_bytes(), &self.commit.signatures) {
            return false;
        }
        return true;
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
        self.public_key.verify(&self.signature, self.hash.as_bytes());
    }
}

