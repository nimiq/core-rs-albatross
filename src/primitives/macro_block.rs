use algebra::bls12_377::{G1Projective, G2Projective};
use algebra::ProjectiveCurve;
use crypto_primitives::FixedLengthCRH;
use nimiq_bls::{KeyPair, PublicKey};

use crate::constants::VALIDATOR_SLOTS;
use crate::primitives::crh::{setup_crh, CRH};

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
    /// This function is only useful for testing purposes.
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

    /// This function is only useful for testing purposes.
    pub fn sign(&mut self, key_pair: &KeyPair, signer_id: usize) {
        self.sign_prepare(key_pair, signer_id);
        self.sign_commit(key_pair, signer_id);
    }

    /// This function is only useful for testing purposes.
    pub fn sign_prepare(&mut self, key_pair: &KeyPair, signer_id: usize) {
        let hash_point = self.hash(0, 0);
        let signature = key_pair.secret_key.sign_g1(hash_point);

        if let Some(sig) = self.prepare_signature.as_mut() {
            *sig += &signature.signature;
        } else {
            self.prepare_signature = Some(signature.signature);
        }

        self.prepare_signer_bitmap[signer_id] = true;
    }

    /// This function is only useful for testing purposes.
    pub fn sign_commit(&mut self, key_pair: &KeyPair, signer_id: usize) {
        let hash_point = self.hash(1, 0);
        let signature = key_pair.secret_key.sign_g1(hash_point);

        if let Some(sig) = self.commit_signature.as_mut() {
            *sig += &signature.signature;
        } else {
            self.commit_signature = Some(signature.signature);
        }

        self.commit_signer_bitmap[signer_id] = true;
    }

    /// This function is only useful for testing purposes.
    pub fn hash(&self, round_number: u8, block_number: u32) -> G1Projective {
        let parameters = setup_crh();

        let mut msg = vec![round_number];
        msg.extend_from_slice(&block_number.to_be_bytes());
        msg.extend_from_slice(&self.header_hash);
        for key in self.public_keys.iter() {
            let pk = PublicKey { public_key: *key };
            msg.extend_from_slice(pk.compress().as_ref());
        }

        println!("hash from off-circuit:\n{:?}\n", msg);

        let result = CRH::evaluate(&parameters, &msg).unwrap();

        println!("g1 from off-circuit:\n{:?}\n", result.into_affine());

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
