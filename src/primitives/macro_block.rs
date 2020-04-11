use algebra::mnt6_753::{G1Projective, G2Projective};
use algebra::ProjectiveCurve;
use crypto_primitives::prf::Blake2sWithParameterBlock;
use nimiq_bls::{KeyPair, PublicKey};

use crate::constants::{sum_generator_g1_mnt6, VALIDATOR_SLOTS};
use crate::primitives::pedersen::{evaluate_pedersen, setup_pedersen};

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
    pub fn sign(&mut self, key_pair: &KeyPair, signer_id: usize, block_number: u32) {
        self.sign_prepare(key_pair, signer_id, block_number);
        self.sign_commit(key_pair, signer_id, block_number);
    }

    /// This function signs a macro block, for only the prepare round, given a validator's
    /// key pair and signer id (which is simply the position in the signer bitmap).
    pub fn sign_prepare(&mut self, key_pair: &KeyPair, signer_id: usize, block_number: u32) {
        // Generate the hash point for the signature.
        let hash_point = self.hash(0, block_number);

        // Generates the signature.
        let signature = key_pair.secret_key.sign_g1(hash_point);

        if let Some(sig) = self.prepare_signature.as_mut() {
            *sig += &signature.signature;
        } else {
            self.prepare_signature = Some(signature.signature);
        }

        // Set the signer id to true.
        self.prepare_signer_bitmap[signer_id] = true;
    }

    /// This function signs a macro block, for only the prepare round, given a validator's
    /// key pair and signer id (which is simply the position in the signer bitmap).
    pub fn sign_commit(&mut self, key_pair: &KeyPair, signer_id: usize, block_number: u32) {
        // Generate the hash point for the signature.
        let hash_point = self.hash(1, block_number);

        // Generates the signature.
        let signature = key_pair.secret_key.sign_g1(hash_point);

        if let Some(sig) = self.commit_signature.as_mut() {
            *sig += &signature.signature;
        } else {
            self.commit_signature = Some(signature.signature);
        }

        // Set the signer id to true.
        self.commit_signer_bitmap[signer_id] = true;
    }

    /// This function hashes the macro block and outputs an EC point. Internally, it first hashes using
    /// the Blake2s to get an output of 256 bits, then we use the Pedersen hash algorithm to transform
    /// those 256 bits into an EC point.
    pub fn hash(&self, round_number: u8, block_number: u32) -> G1Projective {
        // Generates the message to be signed from the block like so:
        // round number || block number || header_hash || public_keys
        // where || means concatenation.
        let mut msg = vec![round_number];
        msg.extend_from_slice(&block_number.to_be_bytes());
        msg.extend_from_slice(&self.header_hash);
        for key in self.public_keys.iter() {
            let pk = PublicKey { public_key: *key };
            msg.extend_from_slice(pk.compress().as_ref());
        }

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
        let hash = blake2s.evaluate(msg.as_ref());

        // Get the generators for the Pedersen hash.
        let generators = setup_pedersen();

        // Calculate the Pedersen hash.
        let result = evaluate_pedersen(generators, hash, sum_generator_g1_mnt6());

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
