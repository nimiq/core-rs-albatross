use ark_crypto_primitives::prf::Blake2sWithParameterBlock;
use ark_ec::ProjectiveCurve;
use ark_ff::Zero;
use ark_mnt6_753::{Fr, G1Projective};

use crate::constants::{POINT_CAPACITY, VALIDATOR_SLOTS};
use crate::primitives::{pedersen_generators, pedersen_hash};
use crate::utils::{bytes_to_bits, serialize_g1_mnt6};

/// A struct representing a macro block in Albatross.
#[derive(Clone)]
pub struct MacroBlock {
    /// This is simply the Blake2b hash of the entire macro block header.
    pub header_hash: [u8; 32],
    /// This is the aggregated signature of the signers for this block, for the first round.
    pub prepare_signature: G1Projective,
    /// This is a bitmap stating which validators signed this block, for the first round.
    pub prepare_signer_bitmap: Vec<bool>,
    /// This is the aggregated signature of the signers for this block, for the second round.
    pub commit_signature: G1Projective,
    /// This is a bitmap stating which validators signed this block, for the second round.
    pub commit_signer_bitmap: Vec<bool>,
}

impl MacroBlock {
    /// This function generates an header that has no signatures or bitmaps.
    pub fn without_signatures(header_hash: [u8; 32]) -> Self {
        MacroBlock {
            header_hash,
            prepare_signature: G1Projective::zero(),
            prepare_signer_bitmap: vec![false; VALIDATOR_SLOTS],
            commit_signature: G1Projective::zero(),
            commit_signer_bitmap: vec![false; VALIDATOR_SLOTS],
        }
    }

    /// This function signs a macro block, for the prepare and commit rounds, given a validator's
    /// key pair and signer id (which is simply the position in the signer bitmap).
    pub fn sign(&mut self, sk: Fr, signer_id: usize, block_number: u32, pks_commitment: Vec<u8>) {
        self.sign_prepare(sk, signer_id, block_number, pks_commitment.clone());
        self.sign_commit(sk, signer_id, block_number, pks_commitment);
    }

    /// This function signs a macro block, for only the prepare round, given a validator's
    /// key pair and signer id (which is simply the position in the signer bitmap).
    pub fn sign_prepare(
        &mut self,
        sk: Fr,
        signer_id: usize,
        block_number: u32,
        pks_commitment: Vec<u8>,
    ) {
        // Generate the hash point for the signature.
        let mut signature = self.hash(0, block_number, pks_commitment);

        // Generates the signature.
        signature *= sk;

        // Adds the signature to the aggregated signature on the block.
        self.prepare_signature += &signature;

        // Set the signer id to true.
        self.prepare_signer_bitmap[signer_id] = true;
    }

    /// This function signs a macro block, for only the prepare round, given a validator's
    /// key pair and signer id (which is simply the position in the signer bitmap).
    pub fn sign_commit(
        &mut self,
        sk: Fr,
        signer_id: usize,
        block_number: u32,
        pks_commitment: Vec<u8>,
    ) {
        // Generate the hash point for the signature.
        let mut signature = self.hash(1, block_number, pks_commitment);

        // Generates the signature.
        signature *= sk;

        // Adds the signature to the aggregated signature on the block.
        self.commit_signature += &signature;

        // Set the signer id to true.
        self.commit_signer_bitmap[signer_id] = true;
    }

    /// A function that calculates the hash for the block from:
    /// round number || block number || header_hash || pks_commitment
    /// where || means concatenation.
    /// First we use the Pedersen commitment to compress the input. Then we serialize the resulting
    /// EC point and hash it with the Blake2s hash algorithm, getting an output of 256 bits. This is
    /// necessary because the Pedersen commitment is not pseudo-random and we need pseudo-randomness
    /// for the BLS signature scheme. Finally we use the Pedersen hash algorithm on those 256 bits
    /// to obtain a single EC point.
    pub fn hash(
        &self,
        round_number: u8,
        block_number: u32,
        pks_commitment: Vec<u8>,
    ) -> G1Projective {
        // Serialize the input into bits.
        let mut bytes = vec![round_number];

        bytes.extend_from_slice(&block_number.to_be_bytes());

        bytes.extend_from_slice(&self.header_hash);

        bytes.extend(&pks_commitment);

        let bits = bytes_to_bits(&bytes);

        // Calculate the Pedersen generators and the sum generator. The formula used for the ceiling
        // division of x/y is (x+y-1)/y.
        let generators_needed = (bits.len() + POINT_CAPACITY - 1) / POINT_CAPACITY + 1;

        let generators = pedersen_generators(generators_needed);

        // Calculate the Pedersen commitment.
        let first_hash = pedersen_hash(bits, generators.clone());

        // Serialize the Pedersen commitment.
        let serialized_commitment = serialize_g1_mnt6(first_hash);

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
        let second_hash = blake2s.evaluate(&serialized_commitment);

        // Convert to bits.
        let bits = bytes_to_bits(&second_hash);

        // Calculate the Pedersen hash.
        let third_hash = pedersen_hash(bits, generators);

        third_hash
    }
}

impl Default for MacroBlock {
    fn default() -> Self {
        MacroBlock {
            header_hash: [0; 32],
            prepare_signature: G1Projective::prime_subgroup_generator(),
            prepare_signer_bitmap: vec![true; VALIDATOR_SLOTS],
            commit_signature: G1Projective::prime_subgroup_generator(),
            commit_signer_bitmap: vec![true; VALIDATOR_SLOTS],
        }
    }
}
