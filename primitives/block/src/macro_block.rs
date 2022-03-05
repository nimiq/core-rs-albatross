use std::fmt;
use std::io;

use thiserror::Error;

use beserial::{Deserialize, Serialize};
use nimiq_collections::bitset::BitSet;
use nimiq_hash::{Blake2sHash, Blake3Hash, Hash, SerializeContent};
use nimiq_nano_primitives::pk_tree_construct;
use nimiq_primitives::policy;
use nimiq_primitives::slots::Validators;
use nimiq_vrf::VrfSeed;

use crate::signed::{Message, PREFIX_TENDERMINT_PROPOSAL};
use crate::tendermint::TendermintProof;

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

/// The struct representing the header of a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
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
    pub parent_hash: Blake3Hash,
    /// The hash of the header of the preceding election macro block.
    pub parent_election_hash: Blake3Hash,
    /// The seed of the block. This is the BLS signature of the seed of the immediately preceding
    /// block (either micro or macro) using the validator key of the block proposer.
    pub seed: VrfSeed,
    /// The extra data of the block. It is simply 32 raw bytes. No planned use.
    #[beserial(len_type(u8, limit = 32))]
    pub extra_data: Vec<u8>,
    /// The root of the Merkle tree of the blockchain state. It just acts as a commitment to the
    /// state.
    pub state_root: Blake3Hash,
    /// The root of the Merkle tree of the body. It just acts as a commitment to the body.
    pub body_root: Blake3Hash,
    /// A merkle root over all of the transactions that happened in the current epoch.
    pub history_root: Blake3Hash,
}

/// The struct representing the body of a Macro block (can be either checkpoint or election).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct MacroBody {
    /// Contains all the information regarding the next validator set, i.e. their validator
    /// public key, their reward address and their assigned validator slots.
    /// Is only Some when the macro block is an election block.
    pub validators: Option<Validators>,
    /// The root of a special Merkle tree over the next validator's voting keys. It is necessary to
    /// verify the zero-knowledge proofs used in the nano-sync.
    /// Is only Some when the macro block is an election block.
    #[beserial(len_type(u8, limit = 96))]
    pub pk_tree_root: Option<Vec<u8>>,
    /// A bitset representing which validator slots had their reward slashed at the time when this
    /// block was produced. It is used later on for reward distribution.
    pub lost_reward_set: BitSet,
    /// A bitset representing which validator slots were prohibited from producing micro blocks or
    /// proposing macro blocks at the time when this block was produced. It is used later on for
    /// reward distribution.
    pub disabled_set: BitSet,
}

impl MacroBlock {
    /// Returns the Blake3 hash of the block header.
    pub fn hash(&self) -> Blake3Hash {
        self.header.hash()
    }

    /// Returns the Blake2s hash of the block header.
    pub fn hash_blake2s(&self) -> Blake2sHash {
        self.header.hash()
    }

    /// Calculates the following function:
    ///     nano_zkp_hash = Blake2s( Blake3(header) || pk_tree_root )
    /// Where `pk_tree_root` is the root of a special Merkle tree containing the BLS public keys of
    /// the validators for the next epoch.
    /// The `pk_tree_root` is necessary for the Nano ZK proofs and needs to be inserted into the
    /// signature for the macro blocks. The easiest way is to calculate this modified hash and then
    /// use it as the signature message.
    /// Also, the final hash is done with Blake2s because the ZKP circuits can only handle Blake2s.
    /// Only election blocks have the `validators` field, which contain the validators for the next
    /// epoch, so for checkpoint blocks the `pk_tree_root` doesn't exist. Then, for checkpoint blocks
    /// this function simply returns:
    ///     nano_zkp_hash = Blake2s( Blake3(header) )
    pub fn nano_zkp_hash(&self) -> Blake2sHash {
        let mut message = self.hash().serialize_to_vec();

        if let Some(validators) = self.get_validators() {
            // Create the tree.
            let mut pk_tree_root = MacroBlock::pk_tree_root(&validators);

            // Add it to the message.
            message.append(&mut pk_tree_root);
        }

        // Return the final hash.
        message.hash()
    }

    /// Calculates the PKTree root from the given validators.
    pub fn pk_tree_root(validators: &Validators) -> Vec<u8> {
        // Get the public keys.
        let public_keys = validators
            .voting_keys()
            .iter()
            .map(|pk| pk.public_key)
            .collect();

        // Create the tree
        pk_tree_construct(public_keys)
    }

    /// Returns whether or not this macro block is an election block.
    pub fn is_election_block(&self) -> bool {
        policy::is_election_block_at(self.header.block_number)
    }

    /// Returns a copy of the validator slots. Only returns Some if it is an election block.
    pub fn get_validators(&self) -> Option<Validators> {
        self.body.as_ref()?.validators.clone()
    }

    /// Returns the block number of this macro block.
    pub fn block_number(&self) -> u32 {
        self.header.block_number
    }

    /// Returns the epoch number of this macro block.
    pub fn epoch_number(&self) -> u32 {
        policy::epoch_at(self.header.block_number)
    }
}

impl Message for MacroHeader {
    const PREFIX: u8 = PREFIX_TENDERMINT_PROPOSAL;
}

impl SerializeContent for MacroHeader {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for MacroHeader {}

impl fmt::Display for MacroHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "#{}.{}:MA:{}",
            self.block_number,
            self.view_number,
            self.hash::<Blake3Hash>().to_short_str(),
        )
    }
}

impl SerializeContent for MacroBody {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> {
        Ok(self.serialize(writer)?)
    }
}

impl fmt::Display for MacroBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        fmt::Display::fmt(&self.header, f)
    }
}

#[allow(clippy::derive_hash_xor_eq)] // TODO: Shouldn't be necessary
impl Hash for MacroBody {}

#[derive(Error, Debug)]
pub enum IntoSlotsError {
    #[error("Body missing in macro block")]
    MissingBody,
    #[error("Not an election macro block")]
    NoElection,
}
