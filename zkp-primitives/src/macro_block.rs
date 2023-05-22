use std::ops::Mul;

use ark_ec::Group;
use ark_mnt6_753::{Fr, G1Projective};
use beserial::Serialize;
use nimiq_bls::Signature;
use nimiq_hash::{Blake2sHash, Hash, SerializeContent};
use nimiq_primitives::policy::Policy;
use num_traits::identities::Zero;

/// A struct representing an election macro block in Albatross.
#[derive(Clone)]
pub struct MacroBlock {
    /// The block number for this block.
    pub block_number: u32,
    /// The Tendermint round number for this block.
    pub round_number: u32,
    /// This is simply the Blake2b hash of the entire macro block header.
    pub header_hash: Blake2sHash,
    /// This is the aggregated signature of the signers for this block.
    pub signature: G1Projective,
    /// This is a bitmap stating which validators signed this block.
    pub signer_bitmap: Vec<bool>,
}

impl MacroBlock {
    pub const TENDERMINT_STEP: u8 = 0x04;

    /// This function generates a macro block that has no signature or bitmap.
    pub fn without_signatures(
        block_number: u32,
        round_number: u32,
        header_hash: Blake2sHash,
    ) -> Self {
        MacroBlock {
            block_number,
            round_number,
            header_hash,
            signature: G1Projective::zero(),
            signer_bitmap: vec![false; Policy::SLOTS as usize],
        }
    }

    /// This function signs a macro block given a validator's secret key and signer id (which is
    /// simply the position in the signer bitmap).
    pub fn sign(&mut self, sk: &Fr, signer_id: usize) {
        // Generate the hash point for the signature.
        let hash_point = self.hash();

        // Generates the signature.
        let signature = hash_point.mul(sk);

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
    pub fn hash(&self) -> G1Projective {
        Signature::hash_to_g1(Hash::hash::<Blake2sHash>(self)) // PITODO take clone away
    }
}

impl Default for MacroBlock {
    fn default() -> Self {
        MacroBlock {
            block_number: 0,
            round_number: 0,
            header_hash: Blake2sHash::default(),
            signature: G1Projective::generator(),
            signer_bitmap: vec![true; Policy::SLOTS as usize],
        }
    }
}
impl Hash for MacroBlock {}

impl SerializeContent for MacroBlock {
    fn serialize_content<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<usize> {
        // First of all serialize step as this also serves as the unique prefix for this message type.
        let mut size = Self::TENDERMINT_STEP.serialize(writer)?;

        // serialize the round number
        size += self.round_number.serialize(writer)?;

        // serialize the block number
        size += self.block_number.serialize(writer)?;

        // serialize the proposal hash
        let proposal_hash = Some(self.header_hash.clone());
        size += proposal_hash.serialize(writer)?;

        // And return the size
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use nimiq_block::{MacroBlock as Block, MacroBody as Body, MultiSignature, TendermintProof};
    use nimiq_bls::{AggregateSignature, KeyPair};
    use nimiq_collections::BitSet;
    use nimiq_keys::{Address, PublicKey as SchnorrPK};
    use nimiq_primitives::{
        policy::Policy,
        slots::{Validator, Validators},
    };
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng::test_rng;
    use nimiq_utils::key_rng::SecureGenerate;

    use super::*;

    #[test]
    fn hash_and_sign_works() {
        let mut rng = test_rng(false);

        // Create validators.
        let mut validator_vec = vec![];
        let mut validator_keys = vec![];

        for i in 0..Policy::SLOTS {
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

        // Get the header hash.
        let header_hash = block.hash_blake2s();

        // Create the zkp_primitives MacroBlock.
        let mut zkp_block = MacroBlock::without_signatures(0, 0, header_hash);

        for (i, sk) in validator_keys.iter().enumerate() {
            zkp_block.sign(sk, i);
        }

        // Create the TendermintProof using our signature.
        let agg_sig = AggregateSignature::from_signatures(&[Signature::from(zkp_block.signature)]);

        let mut bitset = BitSet::with_capacity(Policy::SLOTS as usize);

        for (i, b) in zkp_block.signer_bitmap.iter().enumerate() {
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
