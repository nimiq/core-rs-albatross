use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::G1Var;
use ark_r1cs_std::prelude::{ToBitsGadget, UInt32, UInt8};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::mnt4::{PedersenHashGadget, SerializeGadget};
use crate::utils::reverse_inner_byte_order;

/// This gadget is meant to calculate the "state commitment" in-circuit, which is simply a commitment,
/// for a given block, of the block number concatenated with the root of a Merkle tree over the public
/// keys. We don't calculate the Merkle tree from the public keys. We just serialize the block number
/// and the Merkle tree root and feed it to the Pedersen hash function. Then we serialize the
/// output and convert it to bytes. This provides an efficient way of compressing the state and
/// representing it across different curves.
pub struct StateCommitmentGadget;

impl StateCommitmentGadget {
    /// Calculates the state commitment.
    pub fn evaluate(
        cs: ConstraintSystemRef<MNT4Fr>,
        block_number: &UInt32<MNT4Fr>,
        pks_commitment: &Vec<UInt8<MNT4Fr>>,
        pedersen_generators: &Vec<G1Var>,
    ) -> Result<Vec<UInt8<MNT4Fr>>, SynthesisError> {
        // Initialize Boolean vector.
        let mut bits = vec![];

        // The block number comes in little endian all the way.
        // So, a reverse will put it into big endian.
        let mut block_number_be = block_number.to_bits_le();

        block_number_be.reverse();

        bits.extend(block_number_be);

        // Convert the public keys commitment to bits and append it.
        let mut pks_bits = Vec::new();

        let mut byte;

        for i in 0..pks_commitment.len() {
            byte = pks_commitment[i].to_bits_le()?;
            byte.reverse();
            pks_bits.extend(byte);
        }

        bits.append(&mut pks_bits);

        // Calculate the Pedersen hash.
        let hash = PedersenHashGadget::evaluate(&bits, pedersen_generators)?;

        // Serialize the Pedersen hash.
        let serialized_bits = SerializeGadget::serialize_g1(cs, &hash)?;

        let serialized_bits = reverse_inner_byte_order(&serialized_bits[..]);

        // Convert to bytes.
        let mut bytes = Vec::new();

        for i in 0..serialized_bits.len() / 8 {
            bytes.push(UInt8::from_bits_le(&serialized_bits[i * 8..(i + 1) * 8]));
        }

        Ok(bytes)
    }
}
