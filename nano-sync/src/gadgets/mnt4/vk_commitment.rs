use ark_groth16::constraints::VerifyingKeyVar;
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{G1Var, PairingVar};
use ark_mnt6_753::MNT6_753;
use ark_r1cs_std::prelude::Boolean;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::mnt4::{PedersenHashGadget, SerializeGadget};

/// This gadget is meant to calculate a commitment in-circuit for a verifying key of a SNARK in the
/// MNT6-753 curve. This means we can open this commitment inside of a circuit in the MNT4-753 curve
/// and we can use it to verify a SNARK proof inside that circuit.
/// We calculate it by first serializing the verifying key and feeding it to the Pedersen hash
/// function, then we serialize the output and convert it to bits. This provides an efficient way
/// of compressing the state and representing it across different curves.
pub struct VKCommitmentGadget;

impl VKCommitmentGadget {
    /// Calculates the verifying key commitment.
    pub fn evaluate(
        cs: ConstraintSystemRef<MNT4Fr>,
        vk: &VerifyingKeyVar<MNT6_753, PairingVar>,
        pedersen_generators: &Vec<G1Var>,
    ) -> Result<Vec<Boolean<MNT4Fr>>, SynthesisError> {
        // Initialize Boolean vector.
        let mut bits = vec![];

        // Serialize the verifying key into bits.
        // Alpha G1
        bits.extend(SerializeGadget::serialize_g1(cs.clone(), &vk.alpha_g1)?);

        // Beta G2
        bits.extend(SerializeGadget::serialize_g2(cs.clone(), &vk.beta_g2)?);

        // Gamma G2
        bits.extend(SerializeGadget::serialize_g2(cs.clone(), &vk.gamma_g2)?);

        // Delta G2
        bits.extend(SerializeGadget::serialize_g2(cs.clone(), &vk.delta_g2)?);

        // Gamma ABC G1
        for i in 0..vk.gamma_abc_g1.len() {
            bits.extend(SerializeGadget::serialize_g1(
                cs.clone(),
                &vk.gamma_abc_g1[i],
            )?);
        }

        // Calculate the Pedersen hash.
        let hash = PedersenHashGadget::evaluate(&bits, pedersen_generators)?;

        // Serialize the Pedersen hash.
        let serialized_bits = SerializeGadget::serialize_g1(cs, &hash)?;

        Ok(serialized_bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ark_ec::ProjectiveCurve;
    use ark_groth16::constraints::VerifyingKeyVar;
    use ark_groth16::VerifyingKey;
    use ark_mnt4_753::Fr as MNT4Fr;
    use ark_mnt6_753::constraints::{G1Var, PairingVar};
    use ark_mnt6_753::MNT6_753;
    use ark_mnt6_753::{G1Projective, G2Projective};
    use ark_r1cs_std::prelude::AllocVar;
    use ark_r1cs_std::R1CSVar;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};

    use crate::primitives::{pedersen_generators, vk_commitment};
    use crate::utils::bytes_to_bits;

    #[test]
    fn vk_commitment_test() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

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
        let primitive_comm = bytes_to_bits(&vk_commitment(vk.clone()));

        // Allocate the verifying key in the circuit.
        let vk_var =
            VerifyingKeyVar::<_, PairingVar>::new_witness(cs.clone(), || Ok(vk.clone())).unwrap();

        // Allocate the generators.
        let generators_var =
            Vec::<G1Var>::new_witness(cs.clone(), || Ok(pedersen_generators(14))).unwrap();

        // Evaluate vk commitment using the gadget version.
        let gadget_comm = VKCommitmentGadget::evaluate(cs, &vk_var, &generators_var).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_comm.len(), gadget_comm.len());
        for i in 0..primitive_comm.len() {
            assert_eq!(primitive_comm[i], gadget_comm[i].value().unwrap());
        }
    }
}
