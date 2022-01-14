use ark_ec::ProjectiveCurve;
use ark_ff::fields::PrimeField;
use ark_mnt6_753::{Fr, G1Projective};
use num_traits::identities::Zero;

use nimiq_bls::Signature;
use nimiq_hash::{Blake2sHash, Hash, HashOutput};
use nimiq_primitives::policy::SLOTS;

/// A struct representing an election macro block in Albatross.
#[derive(Clone)]
pub struct MacroBlock {
    /// The block number for this block.
    pub block_number: u32,
    /// The Tendermint round number for this block.
    pub round_number: u32,
    /// This is simply the Blake2b hash of the entire macro block header.
    pub header_hash: [u8; 32],
    /// This is the aggregated signature of the signers for this block.
    pub signature: G1Projective,
    /// This is a bitmap stating which validators signed this block.
    pub signer_bitmap: Vec<bool>,
}

impl MacroBlock {
    /// This function generates a macro block that has no signature or bitmap.
    pub fn without_signatures(block_number: u32, round_number: u32, header_hash: [u8; 32]) -> Self {
        MacroBlock {
            block_number,
            round_number,
            header_hash,
            signature: G1Projective::zero(),
            signer_bitmap: vec![false; SLOTS as usize],
        }
    }

    /// This function signs a macro block given a validator's secret key and signer id (which is
    /// simply the position in the signer bitmap).
    pub fn sign(&mut self, sk: &Fr, signer_id: usize, pk_tree_root: &[u8]) {
        // Generate the hash point for the signature.
        let hash_point = self.hash(pk_tree_root);

        // Generates the signature.
        let signature = hash_point.mul(sk.into_repr());

        // Adds the signature to the aggregated signature on the block.
        self.signature += &signature;

        // Set the signer id to true.
        self.signer_bitmap[signer_id] = true;
    }

    /// A function that calculates the hash point for the block. This should match exactly the hash
    /// point used in validator's signatures. It works like this:
    ///     1. Get the header hash and the pk_tree_root.
    ///     2. Calculate the first hash like so:
    ///             first_hash = Blake2s( header_hash || pk_tree_root )
    ///     3. Calculate the second (and final) hash like so:
    ///             second_hash = Blake2s( 0x04 || round number || block number || 0x01 || first_hash )
    ///        The first four fields (0x04, round number, block number, 0x01) are needed for the
    ///        Tendermint protocol and there is no reason to explain their meaning here.
    ///     4. Finally, we take the second hash and map it to an elliptic curve point using the
    ///        "try-and-increment" method.
    /// The function || means concatenation.
    pub fn hash(&self, pk_tree_root: &[u8]) -> G1Projective {
        let mut first_bytes = self.header_hash.to_vec();

        first_bytes.extend(pk_tree_root);

        let first_hash = first_bytes.hash::<Blake2sHash>();

        let mut second_bytes = vec![0x04];

        second_bytes.extend_from_slice(&self.round_number.to_be_bytes());

        second_bytes.extend_from_slice(&self.block_number.to_be_bytes());

        second_bytes.push(0x01);

        second_bytes.extend_from_slice(first_hash.as_bytes());

        let second_hash = second_bytes.hash::<Blake2sHash>();

        Signature::hash_to_point(second_hash)
    }
}

impl Default for MacroBlock {
    fn default() -> Self {
        MacroBlock {
            block_number: 0,
            round_number: 0,
            header_hash: [0; 32],
            signature: G1Projective::prime_subgroup_generator(),
            signer_bitmap: vec![true; SLOTS as usize],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pk_tree_construct;
    use nimiq_block::{MacroBlock as Block, MacroBody as Body, MultiSignature, TendermintProof};
    use nimiq_bls::{AggregateSignature, KeyPair};
    use nimiq_collections::BitSet;
    use nimiq_keys::{Address, PublicKey as SchnorrPK};
    use nimiq_primitives::policy::SLOTS;
    use nimiq_primitives::slots::{Validator, Validators};
    use nimiq_utils::key_rng::SecureGenerate;

    use super::*;

    #[test]
    fn hash_and_sign_works() {
        let mut rng = rand::thread_rng();

        // Create validators.
        let mut validator_vec = vec![];
        let mut validator_keys = vec![];

        for i in 0..SLOTS {
            let key_pair = KeyPair::generate(&mut rng);

            let val = Validator::new(
                Address::from([i as u8; 20]),
                key_pair.public_key,
                SchnorrPK::default(),
                (i, i + 1),
            );

            validator_vec.push(val);

            validator_keys.push(key_pair.secret_key.secret_key);
        }

        // Create block.
        let mut block = Block::default();

        let mut body = Body::default();

        let validators = Validators::new(validator_vec);

        body.validators = Some(validators.clone());

        block.body = Some(body);

        // Get the pk_tree_root.
        let public_keys = validators
            .voting_keys()
            .iter()
            .map(|pk| pk.public_key)
            .collect();

        let pk_tree_root = pk_tree_construct(public_keys);

        // Get the header hash.
        let header_hash = block.hash();

        // Create the nano_primitives MacroBlock.
        let mut nano_block = MacroBlock::without_signatures(0, 0, header_hash.into());

        for (i, sk) in validator_keys.iter().enumerate() {
            nano_block.sign(sk, i, &pk_tree_root);
        }

        // Create the TendermintProof using our signature.
        let agg_sig = AggregateSignature::from_signatures(&[Signature::from(nano_block.signature)]);

        let mut bitset = BitSet::with_capacity(SLOTS as usize);

        for (i, b) in nano_block.signer_bitmap.iter().enumerate() {
            if *b {
                bitset.insert(i);
            }
        }

        let multisig = MultiSignature {
            signature: agg_sig,
            signers: bitset,
        };

        let proof = TendermintProof {
            round: 0,
            sig: multisig,
        };

        // Finally verify the TendermintProof.
        block.justification = Some(proof);

        assert!(TendermintProof::verify(&block, &validators));
    }
}
