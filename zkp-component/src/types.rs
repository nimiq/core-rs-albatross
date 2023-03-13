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
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;
use nimiq_network_interface::request::{Handle, RequestError};
use nimiq_network_interface::{
    network::Topic,
    request::{RequestCommon, RequestMarker},
};
use nimiq_zkp_primitives::MacroBlock as ZKPMacroBlock;
use parking_lot::RwLock;
use std::borrow::Cow;
use std::path::PathBuf;

use nimiq_zkp_primitives::NanoZKPError;
use thiserror::Error;

use crate::ZKPComponent;

pub const PROOF_GENERATION_OUTPUT_DELIMITER: [u8; 2] = [242, 208];

/// The ZKP event returned by the stream.
#[derive(Debug)]
pub struct ZKPEvent<N: Network> {
    pub source: ProofSource<N>,
    pub proof: ZKProof,
    pub block: MacroBlock,
}

impl<N: Network> ZKPEvent<N> {
    pub fn new(source: ProofSource<N>, proof: ZKProof, block: MacroBlock) -> Self {
        ZKPEvent {
            source,
            proof,
            block,
        }
    }
}

impl<N: Network> Clone for ZKPEvent<N> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            proof: self.proof.clone(),
            block: self.block.clone(),
        }
    }
}

/// The ZKP event returned for individual requests by the ZKP requests component.
#[derive(Debug)]
pub enum ZKPRequestEvent {
    /// A valid proof that has been pushed to the ZKP state.
    Proof { proof: ZKProof, block: MacroBlock },
    /// The peer does not have a more recent proof.
    OutdatedProof { block_height: u32 },
}

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
    pub fn with_genesis(genesis_block: &MacroBlock) -> Result<Self, Error> {
        let latest_pks: Vec<_> = genesis_block
            .get_validators()
            .ok_or(Error::InvalidBlock)?
            .voting_keys()
            .into_iter()
            .map(|pub_key| pub_key.public_key)
            .collect();

        let genesis_block =
            ZKPMacroBlock::try_from(genesis_block).map_err(|_| Error::InvalidBlock)?;

        Ok(ZKPState {
            latest_pks,
            latest_header_hash: genesis_block.header_hash.into(),
            latest_block_number: genesis_block.block_number,
            latest_proof: None,
        })
    }
}

/// The serialization of the ZKPState is unsafe over the network.
/// It uses unchecked serialization of elliptic curve points for performance reasons.
/// We only invoke it when transferring data from the proof generation process.
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
            // Unchecked serialization happening here.
            CanonicalSerialize::serialize_uncompressed(pk, writer.by_ref())
                .map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::uncompressed_size(pk);
        }
        size += Serialize::serialize(&self.latest_header_hash, writer)?;
        size += Serialize::serialize(&self.latest_block_number, writer)?;
        size += Serialize::serialize(&self.latest_proof.is_some(), writer)?;
        if let Some(ref latest_proof) = self.latest_proof {
            CanonicalSerialize::serialize_uncompressed(latest_proof, writer)
                .map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::serialized_size(latest_proof, ark_serialize::Compress::No);
        }
        Ok(size)
    }
    fn serialized_size(&self) -> usize {
        let mut size = 2; // count as u16
        for pk in self.latest_pks.iter() {
            size += CanonicalSerialize::uncompressed_size(pk);
        }
        size += Serialize::serialized_size(&self.latest_header_hash);
        size += Serialize::serialized_size(&self.latest_block_number);
        size += Serialize::serialized_size(&self.latest_proof.is_some());
        if let Some(ref latest_proof) = self.latest_proof {
            size += CanonicalSerialize::serialized_size(latest_proof, ark_serialize::Compress::No);
        }
        size
    }
}

/// The deserialization of the ZKPState is unsafe over the network.
/// It uses unchecked deserialization of elliptic curve points for performance reasons.
/// We only invoke it when transferring data from the proof generation process.
impl Deserialize for ZKPState {
    fn deserialize<R: beserial::ReadBytesExt>(
        reader: &mut R,
    ) -> Result<Self, BeserialSerializingError> {
        let count: u16 = Deserialize::deserialize(reader)?;
        let mut latest_pks: Vec<G2MNT6> = Vec::with_capacity(count as usize);
        for _ in 0..count {
            // Unchecked deserialization happening here.
            latest_pks.push(
                CanonicalDeserialize::deserialize_uncompressed_unchecked(reader.by_ref())
                    .map_err(ark_to_bserial_error)?,
            );
        }
        let latest_header_hash = Deserialize::deserialize(reader)?;
        let latest_block_number = Deserialize::deserialize(reader)?;
        let is_some: bool = Deserialize::deserialize(reader)?;
        let mut latest_proof = None;
        if is_some {
            latest_proof = Some(
                CanonicalDeserialize::deserialize_uncompressed_unchecked(reader)
                    .map_err(ark_to_bserial_error)?,
            );
        }
        Ok(ZKPState {
            latest_pks,
            latest_header_hash,
            latest_block_number,
            latest_proof,
        })
    }
}

/// Contains the id of the source of the newly pushed proof. This object is sent through the network alongside the zk proof.
#[derive(Copy, Debug)]
pub enum ProofSource<N: Network> {
    PeerGenerated(N::PeerId),
    SelfGenerated,
}

impl<N: Network> Clone for ProofSource<N> {
    fn clone(&self) -> Self {
        match self {
            Self::PeerGenerated(peer_id) => Self::PeerGenerated(*peer_id),
            Self::SelfGenerated => Self::SelfGenerated,
        }
    }
}

impl<N: Network> ProofSource<N> {
    pub fn unwrap_peer_id(&self) -> N::PeerId {
        match self {
            Self::PeerGenerated(peer_id) => *peer_id,
            Self::SelfGenerated => panic!("Called unwrap_peer_id on a self generated proof source"),
        }
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
            CanonicalSerialize::serialize_compressed(latest_proof, writer)
                .map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::serialized_size(latest_proof, ark_serialize::Compress::Yes);
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = Serialize::serialized_size(&self.block_number);
        size += Serialize::serialized_size(&self.proof.is_some());
        if let Some(ref latest_proof) = self.proof {
            size += CanonicalSerialize::serialized_size(latest_proof, ark_serialize::Compress::Yes);
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
            latest_proof = Some(
                CanonicalDeserialize::deserialize_compressed(reader)
                    .map_err(ark_to_bserial_error)?,
            );
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
#[derive(Clone, Debug, PartialEq)]
pub struct ProofInput {
    pub block: MacroBlock,
    pub latest_pks: Vec<G2MNT6>,
    pub latest_header_hash: Blake2bHash,
    pub previous_proof: Option<Proof<MNT6_753>>,
    pub genesis_state: [u8; 95],
    pub prover_keys_path: PathBuf,
}

impl Default for ProofInput {
    fn default() -> Self {
        Self {
            block: Default::default(),
            latest_pks: Default::default(),
            latest_header_hash: Default::default(),
            previous_proof: Default::default(),
            genesis_state: [0; 95],
            prover_keys_path: Default::default(),
        }
    }
}

/// The serialization of the ProofInput is unsafe over the network.
/// It uses unchecked serialization of elliptic curve points for performance reasons.
/// We only invoke it when transferring data to the proof generation process.
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
            // Unchecked serialization happening here.
            CanonicalSerialize::serialize_uncompressed(pk, writer.by_ref())
                .map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::uncompressed_size(pk);
        }

        size += Serialize::serialize(&self.latest_header_hash, writer)?;

        size += Serialize::serialize(&self.previous_proof.is_some(), writer)?;
        if let Some(ref latest_proof) = self.previous_proof {
            CanonicalSerialize::serialize_uncompressed(latest_proof, writer.by_ref())
                .map_err(ark_to_bserial_error)?;
            size += CanonicalSerialize::serialized_size(latest_proof, ark_serialize::Compress::No);
        }

        size += Serialize::serialize(&self.genesis_state, writer)?;

        let path_buf = self.prover_keys_path.to_string_lossy().to_string();
        size += SerializeWithLength::serialize::<u16, _>(&path_buf, writer)?;

        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = Serialize::serialized_size(&self.block);
        size += 2; // count as u16
        for pk in self.latest_pks.iter() {
            size += CanonicalSerialize::uncompressed_size(pk);
        }

        size += Serialize::serialized_size(&self.latest_header_hash);

        size += Serialize::serialized_size(&self.previous_proof.is_some());
        if let Some(ref previous_proof) = self.previous_proof {
            size +=
                CanonicalSerialize::serialized_size(previous_proof, ark_serialize::Compress::No);
        }

        size += Serialize::serialized_size(&self.genesis_state);

        let path_buf = self.prover_keys_path.to_string_lossy().to_string();
        size += SerializeWithLength::serialized_size::<u16>(&path_buf);

        size
    }
}

/// The deserialization of the ProofInput is unsafe over the network.
/// It uses unchecked deserialization of elliptic curve points for performance reasons.
/// We only invoke it when transferring data to the proof generation process.
impl Deserialize for ProofInput {
    fn deserialize<R: beserial::ReadBytesExt>(
        reader: &mut R,
    ) -> Result<Self, BeserialSerializingError> {
        let block = Deserialize::deserialize(reader)?;

        let count: u16 = Deserialize::deserialize(reader)?;
        let mut latest_pks: Vec<G2MNT6> = Vec::with_capacity(count as usize);
        for _ in 0..count {
            // Unchecked deserialization happening here.
            latest_pks.push(
                CanonicalDeserialize::deserialize_uncompressed_unchecked(reader.by_ref())
                    .map_err(ark_to_bserial_error)?,
            );
        }

        let latest_header_hash: Blake2bHash = Deserialize::deserialize(reader)?;

        let is_some: bool = Deserialize::deserialize(reader)?;
        let mut previous_proof = None;

        if is_some {
            previous_proof = Some(
                CanonicalDeserialize::deserialize_uncompressed_unchecked(reader.by_ref())
                    .map_err(ark_to_bserial_error)?,
            );
        }

        let genesis_state = Deserialize::deserialize(reader)?;

        let path_buf: String = DeserializeWithLength::deserialize::<u16, _>(reader)?;

        Ok(ProofInput {
            block,
            latest_pks,
            latest_header_hash,
            previous_proof,
            genesis_state,
            prover_keys_path: PathBuf::from(path_buf),
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
pub enum Error {
    #[error("Nano Zkp Error: {0}")]
    NanoZKP(#[from] NanoZKPError),

    #[error("Proof's blocks are not valid")]
    InvalidBlock,

    #[error("Outdated proof")]
    OutdatedProof,

    #[error("Invalid proof")]
    InvalidProof,

    #[error("Request Error: {0}")]
    Request(#[from] RequestError),
}

#[derive(Error, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[repr(u8)]
pub enum ZKProofGenerationError {
    #[error("Nano Zkp Error: {0}")]
    NanoZKP(#[beserial(len_type(u32))] String),

    #[error("Serialization Error: {0}")]
    SerializingError(#[beserial(len_type(u16))] String),

    #[error("Proof's blocks are not valid")]
    InvalidBlock,

    #[error("Channel error")]
    ChannelError,

    #[error("Process launching error: {0}")]
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
    pub(crate) request_election_block: bool,
}

impl RequestCommon for RequestZKP {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 211;
    type Response = RequestZKPResponse;

    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_ZKP;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[repr(u8)]
pub enum RequestZKPResponse {
    #[beserial(discriminant = 1)]
    Proof(ZKProof, Option<MacroBlock>),
    #[beserial(discriminant = 2)]
    Outdated(u32),
}

#[derive(Clone)]
pub(crate) struct ZKPStateEnvironment {
    pub(crate) zkp_state: Arc<RwLock<ZKPState>>,
    pub(crate) blockchain: BlockchainProxy,
}

impl<N: Network> From<&ZKPComponent<N>> for ZKPStateEnvironment {
    fn from(component: &ZKPComponent<N>) -> Self {
        ZKPStateEnvironment {
            zkp_state: Arc::clone(&component.zkp_state),
            blockchain: component.blockchain.clone(),
        }
    }
}

impl<N: Network> Handle<N, RequestZKPResponse, Arc<ZKPStateEnvironment>> for RequestZKP {
    fn handle(&self, _peer_id: N::PeerId, env: &Arc<ZKPStateEnvironment>) -> RequestZKPResponse {
        // First retrieve the ZKP proof and release the lock again.
        let zkp_state = env.zkp_state.read();
        let latest_block_number = zkp_state.latest_block_number;
        if latest_block_number <= self.block_number {
            return RequestZKPResponse::Outdated(latest_block_number);
        }
        let zkp_proof = (*zkp_state).clone().into();
        drop(zkp_state);

        // Then get the corresponding block if necessary.
        let block = if self.request_election_block {
            env.blockchain
                .read()
                .get_block_at(latest_block_number, true)
                .ok()
                .map(|block| block.unwrap_macro())
        } else {
            None
        };
        RequestZKPResponse::Proof(zkp_proof, block)
    }
}
