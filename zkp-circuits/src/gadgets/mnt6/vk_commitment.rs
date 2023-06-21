use ark_groth16::constraints::VerifyingKeyVar;
use ark_mnt6_753::{constraints::PairingVar, Fq as MNT6Fq, MNT6_753};
use ark_r1cs_std::{alloc::AllocVar, uint8::UInt8};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use nimiq_pedersen_generators::DefaultWindow;
use nimiq_zkp_primitives::pedersen_parameters;

use super::DefaultPedersenParametersVar;
use crate::gadgets::{pedersen::PedersenHashGadget, serialize::SerializeGadget};

/// This gadget is meant to calculate a commitment in-circuit for a verifying key of a SNARK in the
/// MNT6-753 curve. This means we can open this commitment inside of a circuit in the MNT4-753 curve
/// and we can use it to verify a SNARK proof inside that circuit.
/// We calculate it by first serializing the verifying key and feeding it to the Pedersen hash
/// function, then we serialize the output and convert it to bits. This provides an efficient way
/// of compressing the state and representing it across different curves.
pub struct VKCommitmentGadget;

pub type VkCommitmentWindow = DefaultWindow;

impl VKCommitmentGadget {
    /// Calculates the verifying key commitment.
    pub fn evaluate(
        cs: ConstraintSystemRef<MNT6Fq>,
        vk: &VerifyingKeyVar<MNT6_753, PairingVar>,
    ) -> Result<Vec<UInt8<MNT6Fq>>, SynthesisError> {
        let pedersen_generators = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters().sub_window::<VkCommitmentWindow>(),
        )?;

        // Initialize Boolean vector.
        let mut bytes = vec![];

        // Serialize the verifying key into bits.
        // Alpha G1
        bytes.extend(vk.alpha_g1.serialize_compressed(cs.clone())?);

        // Beta G2
        bytes.extend(vk.beta_g2.serialize_compressed(cs.clone())?);

        // Gamma G2
        bytes.extend(vk.gamma_g2.serialize_compressed(cs.clone())?);

        // Delta G2
        bytes.extend(vk.delta_g2.serialize_compressed(cs.clone())?);

        // Gamma ABC G1
        for i in 0..vk.gamma_abc_g1.len() {
            bytes.extend(vk.gamma_abc_g1[i].serialize_compressed(cs.clone())?);
        }

        // Calculate the Pedersen hash.
        let hash =
            PedersenHashGadget::<_, _, VkCommitmentWindow>::evaluate(&bytes, &pedersen_generators)?;

        // Serialize the Pedersen hash.
        let serialized_bytes = hash.serialize_compressed(cs)?;

        Ok(serialized_bytes)
    }
}

#[cfg(test)]
mod tests {
    use ark_ec::CurveGroup;
    use ark_groth16::{constraints::VerifyingKeyVar, VerifyingKey};
    use ark_mnt6_753::{
        constraints::PairingVar, Fq as MNT6Fq, G1Projective, G2Projective, MNT6_753,
    };
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use nimiq_test_log::test;
    use nimiq_zkp_primitives::vk_commitment;

    use super::*;

    #[test]
    fn vk_commitment_test() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create verifying key.
        let mut vk = VerifyingKey::<MNT6_753>::default();
        vk.alpha_g1 = G1Projective::rand(rng).into_affine();
        vk.beta_g2 = G2Projective::rand(rng).into_affine();
        vk.gamma_g2 = G2Projective::rand(rng).into_affine();
        vk.delta_g2 = G2Projective::rand(rng).into_affine();
        vk.gamma_abc_g1 = vec![
            G1Projective::rand(rng).into_affine(),
            G1Projective::rand(rng).into_affine(),
        ];

        // Evaluate vk commitment using the primitive version.
        let primitive_comm = vk_commitment(vk.clone());

        // Allocate the verifying key in the circuit.
        let vk_var =
            VerifyingKeyVar::<_, PairingVar>::new_witness(cs.clone(), || Ok(vk.clone())).unwrap();

        // Evaluate vk commitment using the gadget version.
        let gadget_comm = VKCommitmentGadget::evaluate(cs, &vk_var).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_comm.len(), gadget_comm.len());
        for i in 0..primitive_comm.len() {
            assert_eq!(primitive_comm[i], gadget_comm[i].value().unwrap());
        }
    }
}
