use algebra::curves::bls12_377::{G1Projective, G2Projective};
use algebra::fields::bls12_377::Fr;
use algebra::{Group, ProjectiveCurve, Zero};
use nimiq_bls::{KeyPair, PublicKey};
use nimiq_hash::{Blake2sHash, Blake2sHasher, Hasher};

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
    pub const SLOTS: usize = 2; // TODO: Set correctly.

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
    pub fn sign(&mut self, key_pair: &KeyPair, signer_id: usize) {
        self.sign_prepare(key_pair, signer_id);
        self.sign_commit(key_pair, signer_id);
    }

    /// This function is only useful for testing purposes.
    pub fn sign_prepare(&mut self, key_pair: &KeyPair, signer_id: usize) {
        let hash_point = self.hash(0);
        let signature = key_pair.sign_hash(hash_point);

        if let Some(sig) = self.prepare_signature.as_mut() {
            *sig += &signature.signature;
        } else {
            self.prepare_signature = Some(signature.signature);
        }

        self.prepare_signer_bitmap[signer_id] = true;
    }

    /// This function is only useful for testing purposes.
    pub fn sign_commit(&mut self, key_pair: &KeyPair, signer_id: usize) {
        let hash_point = self.hash(1);
        let signature = key_pair.sign_hash(hash_point);

        if let Some(sig) = self.commit_signature.as_mut() {
            *sig += &signature.signature;
        } else {
            self.commit_signature = Some(signature.signature);
        }

        self.commit_signer_bitmap[signer_id] = true;
    }

    pub fn hash(&self, round_number: u8) -> Blake2sHash {
        let mut sum = G2Projective::zero();
        for key in self.public_keys.iter() {
            sum += key;
        }

        let sum_key = PublicKey { public_key: sum };

        // Then, concatenate the prefix || header_hash || sum_pks,
        // with prefix = (round number || block number).
        let mut prefix = vec![round_number];
        prefix.extend_from_slice(&self.block_number.to_be_bytes());
        let mut msg = prefix;
        msg.extend_from_slice(&self.header_hash);
        msg.extend_from_slice(sum_key.compress().as_ref());

        Blake2sHasher::new().chain(&msg).finish()
    }
}

impl Default for MacroBlock {
    fn default() -> Self {
        MacroBlock {
            block_number: 0,
            header_hash: [0; 32],
            // TODO: Replace by a proper number.
            public_keys: vec![
                G2Projective::prime_subgroup_generator().mul(&Fr::from(2149u32));
                MacroBlock::SLOTS
            ],
            prepare_signature: Some(
                G1Projective::prime_subgroup_generator().mul(&Fr::from(2013u32)),
            ),
            prepare_signer_bitmap: vec![true; MacroBlock::SLOTS],
            commit_signature: Some(
                G1Projective::prime_subgroup_generator().mul(&Fr::from(2013u32)),
            ),
            commit_signer_bitmap: vec![true; MacroBlock::SLOTS],
        }
    }
}
