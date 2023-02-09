use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::G1Var;
use ark_r1cs_std::prelude::UInt32;
use ark_r1cs_std::uint8::UInt8;
use ark_r1cs_std::ToBitsGadget;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::endianness::ToBeBytesGadget;
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
        header_hash: &[UInt8<MNT4Fr>],
        pk_tree_root: &[UInt8<MNT4Fr>],
        pedersen_generators: &[G1Var],
    ) -> Result<Vec<UInt8<MNT4Fr>>, SynthesisError> {
        // Initialize Boolean vector.
        let mut bytes = vec![];

        // The block number.
        let block_number_be = block_number.to_bytes_be()?;

        bytes.extend(block_number_be);

        // Append the header hash.
        bytes.extend_from_slice(header_hash);

        // Append the public key tree root.
        bytes.extend_from_slice(pk_tree_root);

        let bits: Vec<_> = bytes
            .into_iter()
            .flat_map(|byte| byte.to_bits_le().unwrap())
            .collect();

        // Calculate the Pedersen hash.
        let hash = PedersenHashGadget::evaluate(&bits, pedersen_generators)?;

        // Serialize the Pedersen hash.
        let serialized_bytes = SerializeGadget::serialize_g1(cs, &hash)?;

        Ok(serialized_bytes)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt4_753::Fr as MNT4Fr;
    use ark_mnt6_753::constraints::G1Var;
    use ark_mnt6_753::G2Projective;
    use ark_r1cs_std::prelude::{AllocVar, UInt32};
    use ark_r1cs_std::R1CSVar;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use rand::RngCore;

    use nimiq_bls::pedersen::pedersen_generators;
    use nimiq_primitives::policy::Policy;
    use nimiq_test_log::test;
    use nimiq_zkp_primitives::{pk_tree_construct, state_commitment};

    use super::*;

    #[test]
    fn state_commitment_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random keys.
        let g2_point = G2Projective::rand(rng);
        let public_keys = vec![g2_point; Policy::SLOTS as usize];

        // Create random block number.
        let block_number = u32::rand(rng);

        // Create random header hash.
        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        // Evaluate state commitment using the primitive version.
        let primitive_comm = &state_commitment(block_number, header_hash, public_keys.clone());

        // Construct the Merkle tree over the public keys.
        let pk_tree_root = pk_tree_construct(public_keys);

        // Allocate the block number in the circuit.
        let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

        // Allocate the header hash in the circuit.
        let header_hash_var =
            Vec::<UInt8<MNT4Fr>>::new_witness(cs.clone(), || Ok(&header_hash[..])).unwrap();

        // Allocate the public key tree root in the circuit.
        let pk_tree_root_var =
            Vec::<UInt8<MNT4Fr>>::new_witness(cs.clone(), || Ok(&pk_tree_root[..])).unwrap();

        // Allocate the generators.
        let generators_var =
            Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(3))).unwrap();

        // Evaluate state commitment using the gadget version.
        let gadget_comm = StateCommitmentGadget::evaluate(
            cs,
            &block_number_var,
            &header_hash_var,
            &pk_tree_root_var,
            &generators_var,
        )
        .unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_comm.len(), gadget_comm.len());
        for i in 0..primitive_comm.len() {
            assert_eq!(primitive_comm[i], gadget_comm[i].value().unwrap());
        }
    }
}
