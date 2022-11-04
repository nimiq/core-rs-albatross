use std::io;
use std::sync::Arc;

use ark_groth16::Proof;
use ark_mnt6_753::{G2Projective as G2MNT6, MNT6_753};

use ark_serialize::{
    CanonicalDeserialize, CanonicalSerialize, SerializationError as ArkSerializingError,
};
use beserial::{
    Deserialize, DeserializeWithLength, Serialize, SerializeWithLength, SerializingError,
    SerializingError as BeserialSerializingError,
};
use nimiq_block::MacroBlock;
use nimiq_database::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_hash::Blake2bHash;
use nimiq_nano_primitives::MacroBlock as ZKPMacroBlock;
use nimiq_network_interface::request::Handle;
use nimiq_network_interface::{
    network::Topic,
    request::{RequestCommon, RequestMarker},
};
use std::borrow::Cow;

use parking_lot::RwLock;

use nimiq_nano_zkp::NanoZKPError;
use thiserror::Error;

pub const PROOF_GENERATION_OUTPUT_DELIMITER: [u8; 2] = [242, 208];

/// The ZK Proof state containing the pks block info and the proof.
/// The genesis block has no zk proof.
#[derive(Clone, Debug, PartialEq)]
pub struct ZKPState {
    pub latest_pks: Vec<G2MNT6>,
    pub latest_header_hash: Blake2bHash,
    pub latest_block_number: u32,
    pub latest_proof: Option<Proof<MNT6_753>>,
}

impl ZKPState {
    pub fn with_genesis(genesis_block: &MacroBlock) -> Result<Self, ZKPComponentError> {
        let latest_pks: Vec<_> = genesis_block
            .get_validators()
            .ok_or(ZKPComponentError::InvalidBlock)?
            .voting_keys()
            .into_iter()
            .map(|pub_key| pub_key.public_key)
            .collect();

        let genesis_block =
            ZKPMacroBlock::try_from(genesis_block).map_err(|_| ZKPComponentError::InvalidBlock)?;

        Ok(ZKPState {
            latest_pks,
            latest_header_hash: genesis_block.header_hash.into(),
            latest_block_number: genesis_block.block_number,
            latest_proof: None,
        })
    }
}

impl Serialize for ZKPState {
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

impl Deserialize for ZKPState {
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
        Ok(ZKPState {
            latest_pks,
            latest_header_hash,
            latest_block_number,
            latest_proof,
        })
    }
}

/// The ZK Proof and the respective block identifier. This object is sent though the network and stored in the zkp db.
#[derive(Clone, Debug, PartialEq)]
pub struct ZKProof {
    pub block_number: u32,
    pub proof: Option<Proof<MNT6_753>>,
}

impl ZKProof {
    pub fn new(block_number: u32, proof: Option<Proof<MNT6_753>>) -> Self {
        Self {
            block_number,
            proof,
        }
    }
}

impl From<ZKPState> for ZKProof {
    fn from(zkp_component_state: ZKPState) -> Self {
        Self {
            block_number: zkp_component_state.latest_block_number,
            proof: zkp_component_state.latest_proof,
        }
    }
}

impl Serialize for ZKProof {
    fn serialize<W: beserial::WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, beserial::SerializingError> {
        let mut size = Serialize::serialize(&self.block_number, writer)?;
        size += Serialize::serialize(&self.proof.is_some(), writer)?;
        if let Some(ref latest_proof) = self.proof {
            CanonicalSerialize::serialize(latest_proof, writer).map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::serialized_size(latest_proof);
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = Serialize::serialized_size(&self.block_number);
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
        let block_number = Deserialize::deserialize(reader)?;
        let is_some: bool = Deserialize::deserialize(reader)?;
        let mut latest_proof = None;

        if is_some {
            latest_proof =
                Some(CanonicalDeserialize::deserialize(reader).map_err(ark_to_bserial_error)?);
        }

        Ok(ZKProof {
            block_number,
            proof: latest_proof,
        })
    }
}

impl AsDatabaseBytes for ZKProof {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        let v = Serialize::serialize_to_vec(&self);
        Cow::Owned(v)
    }
}

impl FromDatabaseValue for ZKProof {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
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

/// The input to the proof generation process.
#[derive(Clone, Debug, Default)]
pub struct ProofInput {
    pub block: MacroBlock,
    pub latest_pks: Vec<G2MNT6>,
    pub latest_header_hash: Blake2bHash,
    pub previous_proof: Option<Proof<MNT6_753>>,
    pub genesis_state: Vec<u8>,
}

impl Serialize for ProofInput {
    fn serialize<W: beserial::WriteBytesExt>(
        &self,
        writer: &mut W,
    ) -> Result<usize, beserial::SerializingError> {
        let mut size = Serialize::serialize(&self.block, writer)?;

        let count: u16 =
            u16::try_from(self.latest_pks.len()).map_err(|_| SerializingError::Overflow)?;
        size += Serialize::serialize(&count, writer)?;
        for pk in self.latest_pks.iter() {
            CanonicalSerialize::serialize(pk, writer.by_ref()).map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::serialized_size(pk);
        }

        size += Serialize::serialize(&self.latest_header_hash, writer)?;

        size += Serialize::serialize(&self.previous_proof.is_some(), writer)?;
        if let Some(ref latest_proof) = self.previous_proof {
            CanonicalSerialize::serialize(latest_proof, writer.by_ref())
                .map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::serialized_size(latest_proof);
        }

        size += SerializeWithLength::serialize::<u8, _>(&self.genesis_state, writer)?;

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = Serialize::serialized_size(&self.block);
        size += 2; // count as u16
        for pk in self.latest_pks.iter() {
            size += CanonicalSerialize::serialized_size(pk);
        }

        size += Serialize::serialized_size(&self.latest_header_hash);

        size += Serialize::serialized_size(&self.previous_proof.is_some());
        if let Some(ref previous_proof) = self.previous_proof {
            size += CanonicalSerialize::serialized_size(previous_proof);
        }

        size += SerializeWithLength::serialized_size::<u8>(&self.genesis_state);

        size
    }
}

impl Deserialize for ProofInput {
    fn deserialize<R: beserial::ReadBytesExt>(
        reader: &mut R,
    ) -> Result<Self, BeserialSerializingError> {
        let block = Deserialize::deserialize(reader)?;

        let count: u16 = Deserialize::deserialize(reader)?;
        let mut latest_pks: Vec<G2MNT6> = Vec::with_capacity(count as usize);
        for _ in 0..count {
            latest_pks.push(
                CanonicalDeserialize::deserialize(reader.by_ref()).map_err(ark_to_bserial_error)?,
            );
        }

        let latest_header_hash: Blake2bHash = Deserialize::deserialize(reader)?;

        let is_some: bool = Deserialize::deserialize(reader)?;
        let mut previous_proof = None;

        if is_some {
            previous_proof = Some(
                CanonicalDeserialize::deserialize(reader.by_ref()).map_err(ark_to_bserial_error)?,
            );
        }

        let genesis_state = DeserializeWithLength::deserialize::<u8, _>(reader)?;

        Ok(ProofInput {
            block,
            latest_pks,
            latest_header_hash,
            previous_proof,
            genesis_state,
        })
    }
}

/// The topic for zkp gossiping.
#[derive(Clone, Debug, Default)]
pub struct ZKProofTopic;

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

#[derive(Error, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[repr(u8)]
pub enum ZKProofGenerationError {
    #[error("Nano Zkp Error")]
    NanoZKP(#[beserial(len_type(u32))] String),

    #[error("Serialization Error")]
    SerializingError(#[beserial(len_type(u16))] String),

    #[error("Proof's blocks are not valid")]
    InvalidBlock,

    #[error("Channel error")]
    ChannelError,

    #[error("Process launching error")]
    ProcessError(#[beserial(len_type(u16))] String),
}

impl From<SerializingError> for ZKProofGenerationError {
    fn from(e: SerializingError) -> Self {
        ZKProofGenerationError::SerializingError(e.to_string())
    }
}

impl From<NanoZKPError> for ZKProofGenerationError {
    fn from(e: NanoZKPError) -> Self {
        ZKProofGenerationError::NanoZKP(e.to_string())
    }
}

impl From<io::Error> for ZKProofGenerationError {
    fn from(e: io::Error) -> Self {
        ZKProofGenerationError::ProcessError(e.to_string())
    }
}

/// The max number of ZKP requests per peer.
pub const MAX_REQUEST_RESPONSE_ZKP: u32 = 1000;

/// The request of a zkp. The request specifies the block height to be used as a filtering mechanism to avoid flooding the network
/// with older proofs.
/// The response should either have a more recent proof (> than block_number) or None.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestZKP {
    pub(crate) block_number: u32,
}

impl RequestCommon for RequestZKP {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 211;
    type Response = Option<ZKProof>;

    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_ZKP;
}

impl Handle<Option<ZKProof>, Arc<RwLock<ZKPState>>> for RequestZKP {
    fn handle(&self, zkp_component: &Arc<RwLock<ZKPState>>) -> Option<ZKProof> {
        let zkp_state = zkp_component.read();
        if zkp_state.latest_block_number > self.block_number {
            return Some((*zkp_state).clone().into());
        }
        None
    }
}
