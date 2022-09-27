use ark_groth16::Proof;
use ark_mnt6_753::{G2Projective as G2MNT6, MNT6_753};

use ark_serialize::{
    CanonicalDeserialize, CanonicalSerialize, SerializationError as ArkSerializingError,
};
use beserial::{
    Deserialize, Serialize, SerializingError, SerializingError as BeserialSerializingError,
};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Topic;
use parking_lot::RwLockReadGuard;

use nimiq_nano_zkp::NanoZKPError;
use thiserror::Error;

pub struct ZKPComponentState {
    pub latest_pks: Vec<G2MNT6>,
    pub latest_header_hash: Blake2bHash,
    pub latest_block_number: u32,
    pub latest_proof: Option<Proof<MNT6_753>>,
}

impl Serialize for ZKPComponentState {
    fn serialize<W: beserial::WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, beserial::SerializingError> {
        let mut size = 0;
        let count: u16 =
            u16::try_from(self.latest_pks.len()).map_err(|_| SerializingError::Overflow)?;
        size += Serialize::serialize(&count, writer)?;
        for pk in self.latest_pks.iter() {
            CanonicalSerialize::serialize(pk, writer.by_ref()).map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::serialized_size(pk);
        }

        size += Serialize::serialize(&self.latest_header_hash, writer)?;
        size += Serialize::serialize(&self.latest_block_number, writer)?;

        size += Serialize::serialize(&self.latest_proof.is_some(), writer)?;
        if let Some(ref latest_proof) = self.latest_proof {
            CanonicalSerialize::serialize(latest_proof, writer).map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::serialized_size(latest_proof);
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 2; // count as u16
        for pk in self.latest_pks.iter() {
            size += CanonicalSerialize::serialized_size(pk);
        }

        size += Serialize::serialized_size(&self.latest_header_hash);
        size += Serialize::serialized_size(&self.latest_block_number);

        size += Serialize::serialized_size(&self.latest_proof.is_some());
        if let Some(ref latest_proof) = self.latest_proof {
            size += CanonicalSerialize::serialized_size(latest_proof);
        }
        size
    }
}

impl Deserialize for ZKPComponentState {
    fn deserialize<R: beserial::ReadBytesExt>(
        reader: &mut R,
    ) -> Result<Self, BeserialSerializingError> {
        let count: u16 = Deserialize::deserialize(reader)?;
        let mut latest_pks: Vec<G2MNT6> = Vec::with_capacity(count as usize);
        for _ in 0..count {
            latest_pks.push(
                CanonicalDeserialize::deserialize(reader.by_ref()).map_err(ark_to_bserial_error)?,
            );
        }

        let latest_header_hash = Deserialize::deserialize(reader)?;
        let latest_block_number = Deserialize::deserialize(reader)?;

        let is_some: bool = Deserialize::deserialize(reader)?;
        let mut latest_proof = None;

        if is_some {
            latest_proof =
                Some(CanonicalDeserialize::deserialize(reader).map_err(ark_to_bserial_error)?);
        }

        Ok(ZKPComponentState {
            latest_pks,
            latest_header_hash,
            latest_block_number,
            latest_proof,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ZKProofTopic;

#[derive(Clone, Debug)]
pub struct ZKProof {
    pub header_hash: Blake2bHash,
    pub proof: Option<Proof<MNT6_753>>,
}

impl ZKProof {
    pub fn new(header_hash: Blake2bHash, proof: Option<Proof<MNT6_753>>) -> Self {
        Self { header_hash, proof }
    }
}

impl From<RwLockReadGuard<'_, ZKPComponentState>> for ZKProof {
    fn from(zkp_component_state: RwLockReadGuard<ZKPComponentState>) -> Self {
        Self {
            header_hash: zkp_component_state.latest_header_hash.clone(),
            proof: zkp_component_state.latest_proof.clone(),
        }
    }
}

impl Serialize for ZKProof {
    fn serialize<W: beserial::WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, beserial::SerializingError> {
        let mut size = Serialize::serialize(&self.header_hash, writer)?;
        size += Serialize::serialize(&self.proof.is_some(), writer)?;
        if let Some(ref latest_proof) = self.proof {
            CanonicalSerialize::serialize(latest_proof, writer).map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::serialized_size(latest_proof);
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = Serialize::serialized_size(&self.header_hash);
        size += Serialize::serialized_size(&self.proof.is_some());
        if let Some(ref latest_proof) = self.proof {
            size += CanonicalSerialize::serialized_size(latest_proof);
        }
        size
    }
}

impl Deserialize for ZKProof {
    fn deserialize<R: beserial::ReadBytesExt>(
        reader: &mut R,
    ) -> Result<Self, BeserialSerializingError> {
        let header_hash = Deserialize::deserialize(reader)?;
        let is_some: bool = Deserialize::deserialize(reader)?;
        let mut latest_proof = None;

        if is_some {
            latest_proof =
                Some(CanonicalDeserialize::deserialize(reader).map_err(ark_to_bserial_error)?);
        }

        Ok(ZKProof {
            header_hash,
            proof: latest_proof,
        })
    }
}

fn ark_to_bserial_error(error: ArkSerializingError) -> BeserialSerializingError {
    match error {
        ArkSerializingError::NotEnoughSpace => BeserialSerializingError::Overflow,
        ArkSerializingError::InvalidData => BeserialSerializingError::InvalidValue,
        ArkSerializingError::UnexpectedFlags => BeserialSerializingError::InvalidValue,
        ArkSerializingError::IoError(e) => BeserialSerializingError::IoError(e),
    }
}

impl Topic for ZKProofTopic {
    type Item = ZKProof;

    const BUFFER_SIZE: usize = 16;
    const NAME: &'static str = "zkproofs";
    const VALIDATE: bool = true;
}

#[derive(Error, Debug)]
pub enum ZKPComponentError {
    #[error("Nano Zkp Error")]
    NanoZKP(#[from] NanoZKPError),

    #[error("Proof's blocks are not valid")]
    InvalidBlock,

    #[error("Outdated proof")]
    OutdatedProof,

    #[error("Invalid proof")]
    InvalidProof,
}
