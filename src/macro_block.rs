use algebra::curves::bls12_377::{G1Projective, G2Projective};
use algebra::ProjectiveCurve;
use crypto_primitives::crh::pedersen::PedersenParameters;
use crypto_primitives::FixedLengthCRH;
use nimiq_bls::{KeyPair, PublicKey};

use crate::constants::VALIDATOR_SLOTS;
use crate::gadgets::crh::{CRHWindow, CRH};

#[derive(Clone)]
pub struct MacroBlock {
    pub header_hash: [u8; 32],
    pub public_keys: Vec<G2Projective>,
    pub prepare_signature: Option<G1Projective>,
    pub prepare_signer_bitmap: Vec<bool>,
    pub commit_signature: Option<G1Projective>,
    pub commit_signer_bitmap: Vec<bool>,
}

impl MacroBlock {
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

    /// Calculates the Pedersen Hash for the block from:
    /// prefix || header_hash || public_keys
    ///
    /// Note that the Pedersen Hash is only collision-resistant
    /// and does not provide pseudo-random output!
    /// For our use-case, however, this suffices as the `header_hash`
    /// provides enough entropy.
    pub fn hash(
        &self,
        round_number: u8,
        block_number: u32,
        parameters: &PedersenParameters<G1Projective>,
    ) -> G1Projective {
        // Then, concatenate the prefix || header_hash || public_keys,
        // with prefix = (round number || block number).
        let mut msg = vec![round_number];
        msg.extend_from_slice(&block_number.to_be_bytes());
        msg.extend_from_slice(&self.header_hash);
        for key in self.public_keys.iter() {
            let pk = PublicKey { public_key: *key };
            msg.extend_from_slice(pk.compress().as_ref());
        }

        CRH::<CRHWindow>::evaluate(parameters, &msg).unwrap()
    }

    /// This function is only useful for testing purposes.
    pub fn sign(
        &mut self,
        key_pair: &KeyPair,
        signer_id: usize,
        parameters: &PedersenParameters<G1Projective>,
    ) {
        self.sign_prepare(key_pair, signer_id, parameters);
        self.sign_commit(key_pair, signer_id, parameters);
    }

    /// This function is only useful for testing purposes.
    pub fn sign_prepare(
        &mut self,
        key_pair: &KeyPair,
        signer_id: usize,
        parameters: &PedersenParameters<G1Projective>,
    ) {
        let hash_point = self.hash(0, 0, parameters);
        let signature = key_pair.secret_key.sign_g1(hash_point);

        if let Some(sig) = self.prepare_signature.as_mut() {
            *sig += &signature.signature;
        } else {
            self.prepare_signature = Some(signature.signature);
        }

        self.prepare_signer_bitmap[signer_id] = true;
    }

    /// This function is only useful for testing purposes.
    pub fn sign_commit(
        &mut self,
        key_pair: &KeyPair,
        signer_id: usize,
        parameters: &PedersenParameters<G1Projective>,
    ) {
        let hash_point = self.hash(1, 0, parameters);
        let signature = key_pair.secret_key.sign_g1(hash_point);

        if let Some(sig) = self.commit_signature.as_mut() {
            *sig += &signature.signature;
        } else {
            self.commit_signature = Some(signature.signature);
        }

        self.commit_signer_bitmap[signer_id] = true;
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
