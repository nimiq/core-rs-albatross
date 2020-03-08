use algebra::curves::bls12_377::{G1Projective, G2Projective};
use algebra::{ProjectiveCurve, Zero};
use crypto_primitives::crh::pedersen::PedersenParameters;
use crypto_primitives::FixedLengthCRH;
use nimiq_bls::{KeyPair, PublicKey};

use crate::setup::CRH;

#[derive(Clone)]
pub struct MacroBlock {
    pub block_number: u32,
    pub header_hash: [u8; 32],
    pub public_keys: Vec<G2Projective>,
    pub prepare_signature: Option<G1Projective>,
    pub prepare_signer_bitmap: Vec<bool>,
    pub commit_signature: Option<G1Projective>,
    pub commit_signer_bitmap: Vec<bool>,
}

impl MacroBlock {
    // TODO: Set correctly.
    pub const SLOTS: usize = 2;

    pub fn without_signatures(
        block_number: u32,
        header_hash: [u8; 32],
        public_keys: Vec<G2Projective>,
    ) -> Self {
        MacroBlock {
            block_number,
            header_hash,
            public_keys,
            prepare_signature: None,
            prepare_signer_bitmap: vec![false; Self::SLOTS],
            commit_signature: None,
            commit_signer_bitmap: vec![false; Self::SLOTS],
        }
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
        let hash_point = self.hash(0, parameters);
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
        let hash_point = self.hash(1, parameters);
        let signature = key_pair.secret_key.sign_g1(hash_point);

        if let Some(sig) = self.commit_signature.as_mut() {
            *sig += &signature.signature;
        } else {
            self.commit_signature = Some(signature.signature);
        }

        self.commit_signer_bitmap[signer_id] = true;
    }

    pub fn hash(
        &self,
        round_number: u8,
        parameters: &PedersenParameters<G1Projective>,
    ) -> G1Projective {
        let mut sum = G2Projective::zero();
        for key in self.public_keys.iter() {
            sum += key;
        }

        let sum_key = PublicKey { public_key: sum };

        // TODO: change from sum_pks to pks
        // Then, concatenate the prefix || header_hash || pks,
        // with prefix = (round number || block number).
        let mut prefix = vec![round_number];
        prefix.extend_from_slice(&self.block_number.to_be_bytes());
        let mut msg = prefix;
        msg.extend_from_slice(&self.header_hash);
        msg.extend_from_slice(sum_key.compress().as_ref());

        CRH::evaluate(parameters, &msg).unwrap()
    }
}

impl Default for MacroBlock {
    fn default() -> Self {
        MacroBlock {
            block_number: 0,
            header_hash: [0; 32],
            public_keys: vec![G2Projective::prime_subgroup_generator(); MacroBlock::SLOTS],
            prepare_signature: Some(G1Projective::prime_subgroup_generator()),
            prepare_signer_bitmap: vec![true; MacroBlock::SLOTS],
            commit_signature: Some(G1Projective::prime_subgroup_generator()),
            commit_signer_bitmap: vec![true; MacroBlock::SLOTS],
        }
    }
}
