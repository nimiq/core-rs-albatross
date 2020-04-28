use algebra::mnt6_753::{Fr, G1Projective, G2Projective};
use algebra::ProjectiveCurve;
use crypto_primitives::prf::Blake2sWithParameterBlock;

use crate::constants::{sum_generator_g1_mnt6, VALIDATOR_SLOTS};
use crate::primitives::{pedersen_commitment, pedersen_generators, pedersen_hash};
use crate::utils::{bytes_to_bits, serialize_g1_mnt6, serialize_g2_mnt6};

/// A struct representing a macro block in Albatross.
#[derive(Clone)]
pub struct MacroBlock {
    /// This is simply the Blake2b hash of the entire macro block header.
    pub header_hash: [u8; 32],
    /// These are the public keys of the new validator list, so the validators that will produce
    /// blocks during the next epoch.
    pub public_keys: Vec<G2Projective>,
    /// This is the aggregated signature of the signers for this block, for the first round.
    pub prepare_signature: Option<G1Projective>,
    /// This is a bitmap stating which validators signed this block, for the first round.
    pub prepare_signer_bitmap: Vec<bool>,
    /// This is the aggregated signature of the signers for this block, for the second round.
    pub commit_signature: Option<G1Projective>,
    /// This is a bitmap stating which validators signed this block, for the second round.
    pub commit_signer_bitmap: Vec<bool>,
}

impl MacroBlock {
    /// This function generates an header that has no signatures or bitmaps.
    pub fn without_signatures(header_hash: [u8; 32], public_keys: Vec<G2Projective>) -> Self {
        MacroBlock {
            header_hash,
            public_keys,
            prepare_signature: None,
            prepare_signer_bitmap: vec![false; VALIDATOR_SLOTS],
            commit_signature: None,
            commit_signer_bitmap: vec![false; VALIDATOR_SLOTS],
        }
    }

    /// This function signs a macro block, for the prepare and commit rounds, given a validator's
    /// key pair and signer id (which is simply the position in the signer bitmap).
    pub fn sign(&mut self, sk: Fr, signer_id: usize, block_number: u32) {
        self.sign_prepare(sk.clone(), signer_id, block_number);
        self.sign_commit(sk, signer_id, block_number);
    }

    /// This function signs a macro block, for only the prepare round, given a validator's
    /// key pair and signer id (which is simply the position in the signer bitmap).
    pub fn sign_prepare(&mut self, sk: Fr, signer_id: usize, block_number: u32) {
        // Generate the hash point for the signature.
        let hash_point = self.hash(0, block_number);

        // Generates the signature.
        let signature = hash_point.mul(sk);

        self.prepare_signature = Some(signature);

        // Set the signer id to true.
        self.prepare_signer_bitmap[signer_id] = true;
    }

    /// This function signs a macro block, for only the prepare round, given a validator's
    /// key pair and signer id (which is simply the position in the signer bitmap).
    pub fn sign_commit(&mut self, sk: Fr, signer_id: usize, block_number: u32) {
        // Generate the hash point for the signature.
        let hash_point = self.hash(1, block_number);

        // Generates the signature.
        let signature = hash_point.mul(sk);

        self.commit_signature = Some(signature);

        // Set the signer id to true.
        self.commit_signer_bitmap[signer_id] = true;
    }

    /// A function that calculates the hash for the block from:
    /// round number || block number || header_hash || public_keys
    /// where || means concatenation.
    /// First we use the Pedersen commitment to compress the input. Then we serialize the resulting
    /// EC point and hash it with the Blake2s hash algorithm, getting an output of 256 bits. This is
    /// necessary because the Pedersen commitment is not pseudo-random and we need pseudo-randomness
    /// for the BLS signature scheme. Finally we use the Pedersen hash algorithm on those 256 bits
    /// to obtain a single EC point.
    pub fn hash(&self, round_number: u8, block_number: u32) -> G1Projective {
        // Serialize the input into bits.
        let mut bytes = vec![round_number];
        bytes.extend_from_slice(&block_number.to_be_bytes());
        bytes.extend_from_slice(&self.header_hash);
        for i in 0..self.public_keys.len() {
            bytes.extend_from_slice(serialize_g2_mnt6(self.public_keys[i]).as_ref());
        }
        let bits = bytes_to_bits(&bytes);

        //Calculate the Pedersen generators and the sum generator. The formula used for the ceiling
        // division of x/y is (x+y-1)/y.
        let generators_needed = (bits.len() + 752 - 1) / 752;
        let generators = pedersen_generators(generators_needed);
        let sum_generator = sum_generator_g1_mnt6();

        // Calculate the Pedersen commitment.
        let pedersen_commitment = pedersen_commitment(bits, generators.clone(), sum_generator);

        // Serialize the Pedersen commitment.
        let serialized_commitment = serialize_g1_mnt6(pedersen_commitment);

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
        let hash = blake2s.evaluate(&serialized_commitment);

        // Convert to bits.
        let bits = bytes_to_bits(&hash);

        // Get the generators for the Pedersen hash.
        let generators = pedersen_generators(256);

        // Calculate the Pedersen hash.
        let result = pedersen_hash(bits, generators, sum_generator_g1_mnt6());

        result
    }
}

impl Default for MacroBlock {
    fn default() -> Self {
        MacroBlock {
            header_hash: [0; 32],
            public_keys: vec![G2Projective::prime_subgroup_generator(); VALIDATOR_SLOTS],
            prepare_signature: Some(G1Projective::prime_subgroup_generator()),
            prepare_signer_bitmap: vec![true; VALIDATOR_SLOTS],
            commit_signature: Some(G1Projective::prime_subgroup_generator()),
            commit_signer_bitmap: vec![true; VALIDATOR_SLOTS],
        }
    }
}
