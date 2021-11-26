use ark_ec::ProjectiveCurve;
use ark_mnt6_753::{Fr, G1Projective};
use num_traits::identities::Zero;

use nimiq_bls::Signature;
use nimiq_hash::{Blake2sHash, Hash};
use nimiq_primitives::policy::SLOTS;

/// A struct representing a macro block in Albatross.
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
    pub fn sign(&mut self, sk: Fr, signer_id: usize, pk_tree_root: Vec<u8>) {
        // Generate the hash point for the signature.
        let mut signature = self.hash(pk_tree_root);

        // Generates the signature.
        signature *= sk;

        // Adds the signature to the aggregated signature on the block.
        self.signature += &signature;

        // Set the signer id to true.
        self.signer_bitmap[signer_id] = true;
    }

    /// A function that calculates the hash for the block from:
    /// step || block number || round number || header_hash || pk_tree_root
    /// where || means concatenation.
    /// This should match exactly the signatures produced by the validators. Step and round number
    /// are fields needed for the Tendermint protocol, there is no reason to explain their meaning
    /// here.
    /// First we hash the input with the Blake2s hash algorithm, getting an output of 256 bits. Then
    /// we use the try-and-increment method on those 256 bits to obtain a single EC point.
    pub fn hash(&self, pk_tree_root: Vec<u8>) -> G1Projective {
        // Serialize the input into bits.
        // TODO: This first byte is the prefix for the precommit messages, it is the
        //       PREFIX_TENDERMINT_COMMIT constant in the nimiq_block crate. We can't
        //       import nimiq_block because of cyclic dependencies. When those constants
        //       get moved to the policy crate, we should import them here.
        let mut bytes = vec![0x04];

        bytes.extend_from_slice(&self.block_number.to_be_bytes());

        bytes.extend_from_slice(&self.round_number.to_be_bytes());

        bytes.extend_from_slice(&self.header_hash);

        bytes.extend(&pk_tree_root);

        // Blake2s hash.
        let hash = bytes.hash::<Blake2sHash>();

        // Hash-to-curve.
        Signature::hash_to_g1(hash)
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
