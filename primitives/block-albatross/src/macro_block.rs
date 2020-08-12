use std::convert::TryInto;
use std::fmt;
use std::io;

use failure::Fail;

use beserial::{Deserialize, Serialize};
use collections::bitset::BitSet;
use hash::{Blake2bHash, Hash, SerializeContent};
use primitives::policy;
use primitives::slot::{Slots, ValidatorSlots};
use vrf::VrfSeed;

use crate::pbft::PbftProof;
use crate::signed;

/// The struct representing a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroBlock {
    /// The header, contains some basic information and commitments to the body and the state.
    pub header: MacroHeader,
    /// The justification, contains all the information needed to verify that the header was signed
    /// by the correct producers.
    pub justification: Option<PbftProof>,
    /// The body of the block.
    pub body: Option<MacroBody>,
}

/// The struct representing the header of a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroHeader {
    /// The version number of the block. Changing this always results in a hard fork.
    pub version: u16,
    /// The number of the block.
    pub block_number: u32,
    /// The view number of this block. It increases whenever a view change happens and resets on
    /// every macro block.
    pub view_number: u32,
    /// The timestamp of the block. It follows the Unix time and has millisecond precision.
    pub timestamp: u64,
    /// The hash of the header of the immediately preceding block (either micro or macro).
    pub parent_hash: Blake2bHash,
    /// The hash of the header of the preceding election macro block.
    pub parent_election_hash: Blake2bHash,
    /// The seed of the block. This is the BLS signature of the seed of the immediately preceding
    /// block (either micro or macro) using the validator key of the block proposer.
    pub seed: VrfSeed,
    /// The extra data of the block. It is simply 32 raw bytes. No planned use.
    #[beserial(len_type(u8, limit = 32))]
    pub extra_data: Vec<u8>,
    /// The root of the Merkle tree of the blockchain state. It just acts as a commitment to the
    /// state.
    pub state_root: Blake2bHash,
    /// The root of the Merkle tree of the body. It just acts as a commitment to the body.
    pub body_root: Blake2bHash,
}

/// The struct representing the body of a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroBody {
    /// Contains all the information regarding the current validator set, i.e. their validator
    /// public key, their reward address and their assigned validator slots.
    /// Is only Some when the macro block is an election block.
    pub validators: Option<ValidatorSlots>,
    /// A bitset representing which validator slots had their reward slashed when this block was
    /// produced. It is used later on for reward distribution.
    pub lost_reward_set: BitSet,
    /// A bitset representing which validator slots were prohibited from producing micro blocks or
    /// proposing macro blocks when this block was produced. It is used later on for reward
    /// distribution.
    pub disabled_set: BitSet,
    /// A merkle root over all of the transactions that happened in the current epoch.
    pub history_root: Blake2bHash,
}

impl MacroBlock {
    /// Returns the hash of the block header.
    pub fn hash(&self) -> Blake2bHash {
        self.header.hash()
    }

    /// Returns whether or not this macro block is an election block.
    pub fn is_election_block(&self) -> bool {
        policy::is_election_block_at(self.header.block_number)
    }

    /// Returns a copy of the validator slots.
    pub fn get_slots(&self) -> Slots {
        self.clone().try_into().unwrap()
    }
}

impl MacroBody {
    /// Creates an empty body.
    pub fn new() -> Self {
        MacroBody {
            validators: None,
            lost_reward_set: BitSet::new(),
            disabled_set: BitSet::new(),
            history_root: Blake2bHash::default(),
        }
    }

    /// Creates the body for a Macro block given the slashed sets. Sets the validators and
    /// history_root fields to default values.
    pub fn from_slashed_sets(lost_reward_set: BitSet, disabled_set: BitSet) -> Self {
        MacroBody {
            validators: None,
            lost_reward_set,
            disabled_set,
            history_root: Blake2bHash::default(),
        }
    }
}

impl signed::Message for MacroHeader {
    const PREFIX: u8 = signed::PREFIX_PBFT_PROPOSAL;
}

impl SerializeContent for MacroHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for MacroHeader {}

impl SerializeContent for MacroBody {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for MacroBody {}

impl fmt::Display for MacroBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "[#{}, view {}, type Macro]",
            self.header.block_number, self.header.view_number
        )
    }
}

#[derive(Clone, Debug, Fail)]
pub enum IntoSlotsError {
    #[fail(display = "Body missing in macro block")]
    MissingBody,
    #[fail(display = "Not an election macro block")]
    NoElection,
}

impl TryInto<Slots> for MacroBlock {
    type Error = IntoSlotsError;

    /// Transforms the validator_slots field of an election Macro block into a Slots struct.
    fn try_into(self) -> Result<Slots, Self::Error> {
        let validator_slots = self
            .body
            .ok_or(IntoSlotsError::MissingBody)?
            .validators
            .ok_or(IntoSlotsError::NoElection)?;

        Ok(Slots::new(validator_slots))
    }
}
