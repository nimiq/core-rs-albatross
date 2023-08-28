use std::{fmt, io};

use ark_ec::Group;
use nimiq_bls::{G2Projective, PublicKey as BlsPublicKey};
use nimiq_collections::bitset::BitSet;
use nimiq_hash::{Blake2bHash, Blake2sHash, Hash, HashOutput, Hasher, SerializeContent};
use nimiq_keys::{Address, PublicKey as SchnorrPublicKey};
use nimiq_primitives::{
    policy::Policy,
    slots_allocation::{Validators, ValidatorsBuilder},
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::reward::RewardTransaction;
use nimiq_vrf::VrfSeed;
use thiserror::Error;

use crate::{
    signed::{Message, PREFIX_TENDERMINT_PROPOSAL},
    tendermint::TendermintProof,
    BlockError,
};

/// The struct representing a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
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
    /// Returns the Blake2b hash of the block header.
    pub fn hash(&self) -> Blake2bHash {
        self.header.hash()
    }

    /// Returns the Blake2s hash of the block header.
    pub fn hash_blake2s(&self) -> Blake2sHash {
        self.header.hash()
    }

    /// Computes the next interlink from self.header.interlink
    pub fn get_next_interlink(&self) -> Result<Vec<Blake2bHash>, BlockError> {
        if !self.is_election_block() {
            return Err(BlockError::InvalidBlockType);
        }
        let mut interlink = self
            .header
            .interlink
            .clone()
            .expect("Election blocks have interlinks");
        let number_hashes_to_update = if self.block_number() == 0 {
            // 0.trailing_zeros() would be 32, thus we need an exception for it
            0
        } else {
            (self.block_number() / Policy::blocks_per_epoch()).trailing_zeros() as usize
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

    /// Returns whether or not this macro block is an election block.
    pub fn is_election_block(&self) -> bool {
        Policy::is_election_block_at(self.header.block_number)
    }

    /// Returns a copy of the validator slots. Only returns Some if it is an election block.
    pub fn get_validators(&self) -> Option<Validators> {
        self.body.as_ref()?.validators.clone()
    }

    /// Returns the block number of this macro block.
    pub fn block_number(&self) -> u32 {
        self.header.block_number
    }

    /// Returns the block number of this macro block.
    pub fn timestamp(&self) -> u64 {
        self.header.timestamp
    }

    /// Return the round of this macro block.
    pub fn round(&self) -> u32 {
        self.header.round
    }

    /// Returns the epoch number of this macro block.
    pub fn epoch_number(&self) -> u32 {
        Policy::epoch_at(self.header.block_number)
    }

    /// Verifies that the block is valid for the given validators.
    pub(crate) fn verify_validators(&self, validators: &Validators) -> Result<(), BlockError> {
        // Verify the Tendermint proof.
        if !TendermintProof::verify(self, validators) {
            warn!(
                %self,
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
            validators,
            ..Default::default()
        };
        let body_root = body.hash();
        MacroBlock {
            header: MacroHeader {
                body_root,
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
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroHeader {
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
    /// The root of the Merkle tree of the body. It just acts as a commitment to the body.
    pub body_root: Blake2sHash,
    /// The root of the trie diff tree proof.
    pub diff_root: Blake2bHash,
    /// A merkle root over all of the transactions that happened in the current epoch.
    pub history_root: Blake2bHash,
}

impl MacroHeader {
    /// Returns the size, in bytes, of a Macro block header. This represents the maximum possible
    /// size since we assume that the extra_data field is completely filled.
    #[allow(clippy::identity_op)]
    pub const MAX_SIZE: usize = 0
        + /*version*/ nimiq_serde::U16_MAX_SIZE
        + /*block_number*/ nimiq_serde::U32_MAX_SIZE
        + /*round*/ nimiq_serde::U32_MAX_SIZE
        + /*timestamp*/ nimiq_serde::U64_MAX_SIZE
        + /*parent_hash*/ Blake2bHash::SIZE
        + /*parent_election_hash*/ Blake2bHash::SIZE
        + /*interlink*/ nimiq_serde::option_max_size(nimiq_serde::vec_max_size(Blake2bHash::SIZE, 32))
        + /*seed*/ VrfSeed::SIZE
        + /*extra_data*/ nimiq_serde::vec_max_size(nimiq_serde::U8_SIZE, 32)
        + /*state_root*/ Blake2bHash::SIZE
        + /*body_root*/ Blake2sHash::SIZE
        + /*diff_root*/ Blake2bHash::SIZE
        + /*history_root*/ Blake2bHash::SIZE;
}

impl Message for MacroHeader {
    const PREFIX: u8 = PREFIX_TENDERMINT_PROPOSAL;
}

impl fmt::Display for MacroHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "#{}:MA:{}",
            self.block_number,
            self.hash::<Blake2bHash>().to_short_str(),
        )
    }
}

impl SerializeContent for MacroHeader {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
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
        self.body_root.serialize_to_writer(writer)?;
        self.history_root.serialize_to_writer(writer)?;

        Ok(())
    }
}

/// The struct representing the body of a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroBody {
    /// Contains all the information regarding the next validator set, i.e. their validator
    /// public key, their reward address and their assigned validator slots.
    /// Is only Some when the macro block is an election block.
    pub validators: Option<Validators>,
    /// A bitset representing which validator slots will be prohibited from producing micro blocks or
    /// proposing macro blocks in the batch following this macro block.
    /// This set is needed for nodes that do not have the state as it is normally computed
    /// inside the staking contract.
    pub next_batch_initial_punished_set: BitSet,
    /// The reward related transactions of this block.
    pub transactions: Vec<RewardTransaction>,
}

impl MacroBody {
    pub(crate) fn verify(&self, is_election: bool) -> Result<(), BlockError> {
        if is_election != self.validators.is_some() {
            return Err(BlockError::InvalidValidators);
        }

        Ok(())
    }
}

impl SerializeContent for MacroBody {
    fn serialize_content<W: io::Write, H: HashOutput>(&self, writer: &mut W) -> io::Result<()> {
        // PITODO: do we need to hash something if None?
        if let Some(ref validators) = self.validators {
            let pk_tree_root = validators.hash::<H>();
            pk_tree_root.serialize_to_writer(writer)?;
        } else {
            0u8.serialize_to_writer(writer)?;
        }

        let punished_set_hash = self
            .next_batch_initial_punished_set
            .serialize_to_vec()
            .hash::<H>();
        punished_set_hash.serialize_to_writer(writer)?;

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
