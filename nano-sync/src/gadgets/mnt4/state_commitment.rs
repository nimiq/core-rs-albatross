use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::G1Var;
use ark_r1cs_std::bits::boolean::Boolean;
use ark_r1cs_std::prelude::UInt32;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::mnt4::{PedersenHashGadget, SerializeGadget};

/// This gadget is meant to calculate the "state commitment" in-circuit, which is simply a commitment,
/// for a given block, of the block number concatenated with the root of a Merkle tree over the public
/// keys. We don't calculate the Merkle tree from the public keys. We just serialize the block number
/// and the Merkle tree root, feed it to the Pedersen hash function and serialize the output. This
/// provides an efficient way of compressing the state and representing it across different curves.
pub struct StateCommitmentGadget;

impl StateCommitmentGadget {
    /// Calculates the state commitment.
    pub fn evaluate(
        cs: ConstraintSystemRef<MNT4Fr>,
        block_number: &UInt32<MNT4Fr>,
        pk_tree_root: &Vec<Boolean<MNT4Fr>>,
        pedersen_generators: &Vec<G1Var>,
    ) -> Result<Vec<Boolean<MNT4Fr>>, SynthesisError> {
        // Initialize Boolean vector.
        let mut bits = vec![];

        // The block number comes in little endian all the way.
        // So, a reverse will put it into big endian.
        let mut block_number_be = block_number.to_bits_le();

        block_number_be.reverse();

        bits.extend(block_number_be);

        // Append the public key tree root.
        bits.extend_from_slice(pk_tree_root);

        // Calculate the Pedersen hash.
        let hash = PedersenHashGadget::evaluate(&bits, pedersen_generators)?;

        // Serialize the Pedersen hash.
        let serialized_bits = SerializeGadget::serialize_g1(cs, &hash)?;

        Ok(serialized_bits)
    }
}
