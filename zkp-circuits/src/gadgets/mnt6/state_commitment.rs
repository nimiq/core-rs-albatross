use ark_mnt6_753::Fq as MNT6Fq;
use ark_r1cs_std::{prelude::UInt32, uint8::UInt8};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use nimiq_pedersen_generators::GenericWindow;

use super::DefaultPedersenParametersVar;
use crate::gadgets::{
    be_bytes::ToBeBytesGadget, pedersen::PedersenHashGadget, serialize::SerializeGadget,
};

/// This gadget is meant to calculate the "state commitment" in-circuit, which is simply a commitment,
/// for a given block, of the block number concatenated with the root of a Merkle tree over the public
/// keys. We don't calculate the Merkle tree from the public keys. We just serialize the block number
/// and the Merkle tree root, feed it to the Pedersen hash function and serialize the output. This
/// provides an efficient way of compressing the state and representing it across different curves.
pub struct StateCommitmentGadget;

pub type StateCommitmentWindow = GenericWindow<4, MNT6Fq>; // can be reduced to 2

impl StateCommitmentGadget {
    /// Calculates the state commitment.
    pub fn evaluate(
        cs: ConstraintSystemRef<MNT6Fq>,
        block_number: &UInt32<MNT6Fq>,
        header_hash: &[UInt8<MNT6Fq>],
        pk_tree_root: &[UInt8<MNT6Fq>],
        pedersen_generators: &DefaultPedersenParametersVar,
    ) -> Result<Vec<UInt8<MNT6Fq>>, SynthesisError> {
        // Initialize Boolean vector.
        let mut bytes = vec![];

        // The block number.
        let block_number_be = block_number.to_bytes_be()?;

        bytes.extend(block_number_be);

        // Append the header hash.
        bytes.extend_from_slice(header_hash);

        // Append the public key tree root.
        bytes.extend_from_slice(pk_tree_root);

        // Calculate the Pedersen hash.
        let hash = PedersenHashGadget::<_, _, StateCommitmentWindow>::evaluate(
            &bytes,
            pedersen_generators,
        )?;

        // Serialize the Pedersen hash.
        let serialized_bytes = hash.serialize_compressed(cs)?;

        Ok(serialized_bytes)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt6_753::{Fq as MNT6Fq, G2Projective};
    use ark_r1cs_std::{
        prelude::{AllocVar, UInt32},
        R1CSVar,
    };
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use nimiq_pedersen_generators::pedersen_generator_powers;
    use nimiq_primitives::policy::Policy;
    use nimiq_test_log::test;
    use nimiq_zkp_primitives::{pk_tree_construct, state_commitment};
    use rand::RngCore;

    use super::*;
    use crate::gadgets::pedersen::PedersenParametersVar;

    #[test]
    fn state_commitment_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

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

        // Construct the Merkle tree over the public keys.
        let pk_tree_root = pk_tree_construct(public_keys);

        // Evaluate state commitment using the primitive version.
        let primitive_comm = &state_commitment(block_number, &header_hash, &pk_tree_root);

        // Allocate the block number in the circuit.
        let block_number_var = UInt32::new_witness(cs.clone(), || Ok(block_number)).unwrap();

        // Allocate the header hash in the circuit.
        let header_hash_var =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&header_hash[..])).unwrap();

        // Allocate the public key tree root in the circuit.
        let pk_tree_root_var =
            Vec::<UInt8<MNT6Fq>>::new_witness(cs.clone(), || Ok(&pk_tree_root[..])).unwrap();

        // Allocate the generators.
        let parameters = pedersen_generator_powers::<StateCommitmentWindow>();
        let generators_var =
            PedersenParametersVar::new_witness(cs.clone(), || Ok(&parameters)).unwrap();

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
