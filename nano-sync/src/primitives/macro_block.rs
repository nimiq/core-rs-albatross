use ark_crypto_primitives::prf::Blake2sWithParameterBlock;
use ark_ec::ProjectiveCurve;
use ark_ff::Zero;
use ark_mnt6_753::{Fr, G1Projective};

use crate::constants::VALIDATOR_SLOTS;
use crate::primitives::{pedersen_generators, pedersen_hash};
use crate::utils::bytes_to_bits;

/// A struct representing a macro block in Albatross.
// TODO: Add a test to this primitive to make sure that it conforms to the macro block specs.
#[derive(Clone)]
pub struct MacroBlock {
    /// This is simply the Blake2b hash of the entire macro block header.
    pub header_hash: [u8; 32],
    /// This is the aggregated signature of the signers for this block.
    pub signature: G1Projective,
    /// This is a bitmap stating which validators signed this block.
    pub signer_bitmap: Vec<bool>,
}

impl MacroBlock {
    /// This function generates an header that has no signatures or bitmaps.
    pub fn without_signatures(header_hash: [u8; 32]) -> Self {
        MacroBlock {
            header_hash,
            signature: G1Projective::zero(),
            signer_bitmap: vec![false; VALIDATOR_SLOTS],
        }
    }

    /// This function signs a macro block given a validator's key pair and signer id (which is
    /// simply the position in the signer bitmap).
    pub fn sign(
        &mut self,
        sk: Fr,
        signer_id: usize,
        block_number: u32,
        round_number: u32,
        pks_commitment: Vec<u8>,
    ) {
        // Generate the hash point for the signature.
        let mut signature = self.hash(block_number, round_number, pks_commitment);

        // Generates the signature.
        signature *= sk;

        // Adds the signature to the aggregated signature on the block.
        self.signature += &signature;

        // Set the signer id to true.
        self.signer_bitmap[signer_id] = true;
    }

    /// A function that calculates the hash for the block from:
    /// step || block number || round number || header_hash || pks_commitment
    /// where || means concatenation.
    /// This should match exactly the signatures produced by the validators. Step and round number
    /// are fields needed for the Tendermint protocol, there is no reason to explain their meaning
    /// here.
    /// First we hash the input with the Blake2s hash algorithm, getting an output of 256 bits. This
    /// is necessary because the Pedersen commitment is not pseudo-random and we need pseudo-randomness
    /// for the BLS signature scheme. Then we use the Pedersen hash algorithm on those 256 bits
    /// to obtain a single EC point.
    pub fn hash(
        &self,
        block_number: u32,
        round_number: u32,
        pks_commitment: Vec<u8>,
    ) -> G1Projective {
        // Serialize the input into bits.
        // TODO: This first byte is the prefix for the precommit messages, it is the
        //       PREFIX_TENDERMINT_COMMIT constant in the nimiq_block_albatross crate. We can't
        //       import nimiq_block_albatross because of cyclic dependencies. When those constants
        //       get moved to the policy crate, we should import them here.
        let mut bytes = vec![0x04];

        bytes.extend_from_slice(&block_number.to_be_bytes());

        bytes.extend_from_slice(&round_number.to_be_bytes());

        bytes.extend_from_slice(&self.header_hash);

        bytes.extend(&pks_commitment);

        // Initialize Blake2s parameters.
        let blake2s = Blake2sWithParameterBlock {
            digest_length: 32,
            key_length: 0,
            fan_out: 1,
            depth: 1,
            leaf_length: 0,
            node_offset: 0,
            xof_digest_length: 0,
            node_depth: 0,
            inner_length: 0,
            salt: [0; 8],
            personalization: [0; 8],
        };

        // Calculate the Blake2s hash.
        let first_hash = blake2s.evaluate(&bytes);

        // Convert to bits.
        let bits = bytes_to_bits(&first_hash);

        // Calculate the Pedersen generators and the sum generator. The formula used for the ceiling
        // division of x/y is (x+y-1)/y.
        let capacity = 752;

        let generators_needed = (bits.len() + capacity - 1) / capacity + 1;

        let generators = pedersen_generators(generators_needed);

        // Calculate the Pedersen hash.
        let second_hash = pedersen_hash(bits, generators);

        second_hash
    }
}

impl Default for MacroBlock {
    fn default() -> Self {
        MacroBlock {
            header_hash: [0; 32],
            signature: G1Projective::prime_subgroup_generator(),
            signer_bitmap: vec![true; VALIDATOR_SLOTS],
        }
    }
}
