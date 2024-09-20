use std::{fmt, io};

use ark_ec::Group;
use nimiq_bls::{G2Projective, PublicKey as BlsPublicKey};
use nimiq_collections::bitset::BitSet;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash, HashOutput, Hasher, SerializeContent};
use nimiq_keys::{Address, Ed25519PublicKey as SchnorrPublicKey};
use nimiq_primitives::{
    networks::NetworkId,
    policy::Policy,
    slots_allocation::{Validators, ValidatorsBuilder},
    Message, PREFIX_TENDERMINT_PROPOSAL,
};
use nimiq_serde::{Deserialize, Serialize, SerializedMaxSize, SerializedSize};
use nimiq_transaction::reward::RewardTransaction;
use nimiq_vrf::VrfSeed;
use thiserror::Error;

use crate::{tendermint::TendermintProof, BlockError};

/// The struct representing a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize)]
pub struct MacroBlock {
    /// The header, contains some basic information and commitments to the body and the state.
    pub header: MacroHeader,
    /// The body of the block.
    pub body: Option<MacroBody>,
    /// The justification, contains all the information needed to verify that the header was signed
    /// by the correct producers.
    pub justification: Option<TendermintProof>,
}

impl MacroBlock {
    /// Returns the network ID of this macro block.
    pub fn network(&self) -> NetworkId {
        self.header.network
    }

    /// Returns the Blake2b hash of the block header.
    pub fn hash(&self) -> Blake2bHash {
        self.header.hash()
    }

    /// Returns the Blake2s hash of the block header.
    pub fn hash_blake2s(&self) -> Blake2sHash {
        Hash::hash(&self.header)
    }

    /// Computes the next interlink from self.header.interlink
    pub fn get_next_interlink(&self) -> Result<Vec<Blake2bHash>, BlockError> {
        if !self.is_election() {
            return Err(BlockError::InvalidBlockType);
        }
        let mut interlink = self
            .header
            .interlink
            .clone()
            .expect("Election blocks have interlinks");
        let number_hashes_to_update = if self.block_number() == Policy::genesis_block_number() {
            // 0.trailing_zeros() would be 32, thus we need an exception for it
            0
        } else {
            ((self.block_number() - Policy::genesis_block_number()) / Policy::blocks_per_epoch())
                .trailing_zeros() as usize
        };
        if number_hashes_to_update > interlink.len() {
            interlink.push(self.hash());
        }
        assert!(
            interlink.len() >= number_hashes_to_update,
            "{} {}",
            interlink.len(),
            number_hashes_to_update,
        );
        #[allow(clippy::needless_range_loop)]
        for i in 0..number_hashes_to_update {
            interlink[i] = self.hash();
        }
        Ok(interlink)
    }

    /// Returns whether this macro block is an election block.
    pub fn is_election(&self) -> bool {
        self.header.is_election()
    }

    /// Returns a copy of the validator slots. Only returns Some if it is an election block.
    pub fn get_validators(&self) -> Option<Validators> {
        self.header.validators.clone()
    }

    /// Returns the block number of this macro block.
    pub fn block_number(&self) -> u32 {
        self.header.block_number
    }

    /// Returns the batch number of this macro block.
    pub fn batch_number(&self) -> u32 {
        Policy::batch_at(self.header.block_number)
    }

    /// Returns the epoch number of this macro block.
    pub fn epoch_number(&self) -> u32 {
        Policy::epoch_at(self.header.block_number)
    }

    /// Returns the block number of this macro block.
    pub fn timestamp(&self) -> u64 {
        self.header.timestamp
    }

    /// Return the round of this macro block.
    pub fn round(&self) -> u32 {
        self.header.round
    }

    /// Verifies that the block is valid for the given validators.
    pub(crate) fn verify_validators(&self, validators: &Validators) -> Result<(), BlockError> {
        // Verify the Tendermint proof.
        if !TendermintProof::verify(self, validators) {
            warn!(
                block = %self,
                reason = "Macro block with bad justification",
                "Rejecting block"
            );
            return Err(BlockError::InvalidJustification);
        }

        Ok(())
    }

    /// Creates a default block that has body and justification.
    pub fn non_empty_default() -> Self {
        let mut validators = ValidatorsBuilder::new();
        for _ in 0..Policy::SLOTS {
            validators.push(
                Address::default(),
                BlsPublicKey::new(G2Projective::generator()).compress(),
                SchnorrPublicKey::default(),
            );
        }

        let validators = Some(validators.build());
        let body = MacroBody {
            ..Default::default()
        };
        let body_root = body.hash();
        MacroBlock {
            header: MacroHeader {
                body_root,
                validators,
                ..Default::default()
            },
            body: Some(body),
            justification: Some(TendermintProof {
                round: 0,
                sig: Default::default(),
            }),
        }
    }
}

impl fmt::Display for MacroBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(&self.header, f)
    }
}

/// The struct representing the header of a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MacroHeader {
    /// Network of the block.
    pub network: NetworkId,
    /// The version number of the block. Changing this always results in a hard fork.
    pub version: u16,
    /// The number of the block.
    pub block_number: u32,
    /// The round number this block was proposed in.
    pub round: u32,
    /// The timestamp of the block. It follows the Unix time and has millisecond precision.
    pub timestamp: u64,
    /// The hash of the header of the immediately preceding block (either micro or macro).
    pub parent_hash: Blake2bHash,
    /// The hash of the header of the preceding election macro block.
    pub parent_election_hash: Blake2bHash,
    /// Hashes of the last blocks divisible by 2^x
    pub interlink: Option<Vec<Blake2bHash>>,
    /// The seed of the block. This is the BLS signature of the seed of the immediately preceding
    /// block (either micro or macro) using the validator key of the block proposer.
    pub seed: VrfSeed,
    /// The extra data of the block. It is simply up to 32 raw bytes.
    ///
    /// It encodes the initial supply in the genesis block, as a big-endian `u64`.
    ///
    /// No planned use otherwise.
    pub extra_data: Vec<u8>,
    /// The root of the Merkle tree of the blockchain state. It just acts as a commitment to the
    /// state.
    pub state_root: Blake2bHash,
    /// The hash of the body. It just acts as a commitment to the body.
    pub body_root: Blake2sHash,
    /// The root of the trie diff tree proof.
    pub diff_root: Blake2bHash,
    /// A merkle root over all the transactions that happened in the current epoch.
    pub history_root: Blake2bHash,
    /// Contains all the information regarding the next validator set, i.e. their validator
    /// public key, their reward address and their assigned validator slots.
    /// Is only Some when the macro block is an election block.
    pub validators: Option<Validators>,
    /// A bitset representing which validator slots will be prohibited from producing micro blocks or
    /// proposing macro blocks in the batch following this macro block.
    /// This set is needed for nodes that do not have the state as it is normally computed
    /// inside the staking contract.
    pub next_batch_initial_punished_set: BitSet,
    /// The cached hash of this header. This is NOT sent over the wire.
    #[serde(skip)]
    pub cached_hash: Option<Blake2bHash>,
}

impl MacroHeader {
    /// Returns the Blake2b hash of this header.
    pub fn hash(&self) -> Blake2bHash {
        if let Some(hash) = &self.cached_hash {
            return hash.clone();
        }
        Hash::hash(&self)
    }

    /// Returns the Blake2b hash of this header and caches the result internally.
    pub fn hash_cached(&mut self) -> Blake2bHash {
        if self.cached_hash.is_none() {
            self.cached_hash = Some(Hash::hash(self));
        }
        self.cached_hash.as_ref().unwrap().clone()
    }

    /// Returns whether this macro block is an election block.
    pub fn is_election(&self) -> bool {
        Policy::is_election_block_at(self.block_number)
    }

    pub(crate) fn verify(&self) -> Result<(), BlockError> {
        // Check that validators are only set on election blocks.
        if self.is_election() != self.validators.is_some() {
            return Err(BlockError::InvalidValidators);
        }
        Ok(())
    }

    fn zkp_body_root(&self) -> Blake2sHash {
        let mut h = <Blake2sHash as HashOutput>::Builder::default();
        self.zkp_body_serialize_content(&mut h).unwrap();
        h.finish()
    }

    pub fn zkp_body_serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        if let Some(ref validators) = self.validators {
            let pk_tree_root = validators.hash::<Blake2sHash>();
            pk_tree_root.serialize_to_writer(writer)?;
        } else {
            0u8.serialize_to_writer(writer)?;
        }

        let punished_set_hash = self
            .next_batch_initial_punished_set
            .serialize_to_vec()
            .hash::<Blake2sHash>();
        punished_set_hash.serialize_to_writer(writer)?;

        self.body_root.serialize_to_writer(writer)?;

        Ok(())
    }
}

impl SerializedMaxSize for MacroHeader {
    #[allow(clippy::identity_op)]
    const MAX_SIZE: usize = 0
        + /*network*/ NetworkId::SIZE
        + /*version*/ u16::MAX_SIZE
        + /*block_number*/ u32::MAX_SIZE
        + /*round*/ u32::MAX_SIZE
        + /*timestamp*/ u64::MAX_SIZE
        + /*parent_hash*/ Blake2bHash::SIZE
        + /*parent_election_hash*/ Blake2bHash::SIZE
        + /*interlink*/ nimiq_serde::option_max_size(nimiq_serde::seq_max_size(Blake2bHash::SIZE, 32))
        + /*seed*/ VrfSeed::SIZE
        + /*extra_data*/ nimiq_serde::seq_max_size(u8::SIZE, 32)
        + /*state_root*/ Blake2bHash::SIZE
        + /*body_root*/ Blake2sHash::SIZE
        + /*diff_root*/ Blake2bHash::SIZE
        + /*history_root*/ Blake2bHash::SIZE
        + /*validators*/ nimiq_serde::option_max_size(Validators::MAX_SIZE)
        + /*next_batch_punished_set*/ BitSet::max_size(Policy::SLOTS as usize);
}

// We can't derive this because we want to ignore the `cached_hash` field.
impl PartialEq for MacroHeader {
    fn eq(&self, other: &Self) -> bool {
        self.network == other.network
            && self.version == other.version
            && self.block_number == other.block_number
            && self.round == other.round
            && self.timestamp == other.timestamp
            && self.parent_hash == other.parent_hash
            && self.parent_election_hash == other.parent_election_hash
            && self.interlink == other.interlink
            && self.seed == other.seed
            && self.extra_data == other.extra_data
            && self.state_root == other.state_root
            && self.body_root == other.body_root
            && self.diff_root == other.diff_root
            && self.history_root == other.history_root
            && self.validators == other.validators
            && self.next_batch_initial_punished_set == other.next_batch_initial_punished_set
    }
}

impl Eq for MacroHeader {}

impl Message for MacroHeader {
    const PREFIX: u8 = PREFIX_TENDERMINT_PROPOSAL;
}

impl fmt::Display for MacroHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "#{}:MA:{}",
            self.block_number,
            self.hash().to_short_str(),
        )
    }
}

// This needs to be kept in sync with `MacroBlockGadget::hash` of
// `nimiq-zkp-circuits`. Whenever this is changed, `MacroBlockGadget::hash`
// also needs to be adjusted.
impl SerializeContent for MacroHeader {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        self.network.serialize_to_writer(writer)?;
        self.version.to_be_bytes().serialize_to_writer(writer)?;
        self.block_number.to_be_bytes().serialize(writer)?;
        self.round.to_be_bytes().serialize_to_writer(writer)?;

        self.timestamp.to_be_bytes().serialize_to_writer(writer)?;
        self.parent_hash.serialize_to_writer(writer)?;
        self.parent_election_hash.serialize_to_writer(writer)?;

        let interlink_hash = H::Builder::default()
            .chain(&self.interlink.serialize_to_vec())
            .finish();
        interlink_hash.serialize_to_writer(writer)?;

        self.seed.serialize_to_writer(writer)?;

        let extra_data_hash = H::Builder::default()
            .chain(&self.extra_data.serialize_to_vec())
            .finish();
        extra_data_hash.serialize_to_writer(writer)?;

        self.state_root.serialize_to_writer(writer)?;
        // This includes validators and next_batch_punished_set.
        self.zkp_body_root().serialize_to_writer(writer)?;
        self.history_root.serialize_to_writer(writer)?;

        Ok(())
    }
}

/// The struct representing the body of a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, SerializedMaxSize)]
pub struct MacroBody {
    /// The reward related transactions of this block.
    #[serialize_size(seq_max_elems = Policy::SLOTS as usize)]
    pub transactions: Vec<RewardTransaction>,
}

impl SerializeContent for MacroBody {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        let transactions_hash = self.transactions.serialize_to_vec().hash::<H>();
        transactions_hash.serialize_to_writer(writer)?;
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum IntoSlotsError {
    #[error("Body missing in macro block")]
    MissingBody,
    #[error("Not an election macro block")]
    NoElection,
}

#[cfg(test)]
mod test {
    use super::MacroBlock;

    #[test]
    fn size_well_below_msg_limit() {
        use nimiq_serde::SerializedMaxSize;
        assert!(
            2 * dbg!(MacroBlock::MAX_SIZE) + 16384
                <= dbg!(nimiq_network_interface::network::MIN_SUPPORTED_MSG_SIZE)
        );
    }
}
