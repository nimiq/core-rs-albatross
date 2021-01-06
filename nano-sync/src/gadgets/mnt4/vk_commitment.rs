use ark_groth16::constraints::VerifyingKeyVar;
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{G1Var, PairingVar};
use ark_mnt6_753::MNT6_753;
use ark_r1cs_std::prelude::UInt8;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::mnt4::{PedersenHashGadget, SerializeGadget};
use crate::utils::reverse_inner_byte_order;

/// This gadget is meant to calculate a commitment in-circuit for a verifying key of a SNARK in the
/// MNT6-753 curve. This means we can open this commitment inside of a circuit in the MNT4-753 curve
/// and we can use it to verify a SNARK proof inside that circuit.
/// We calculate it by first serializing the verifying key and feeding it to the Pedersen hash
/// function, then we serialize the output and convert it to bytes. This provides an efficient way
/// of compressing the state and representing it across different curves.
pub struct VKCommitmentGadget;

impl VKCommitmentGadget {
    /// Calculates the verifying key commitment.
    pub fn evaluate(
        cs: ConstraintSystemRef<MNT4Fr>,
        vk: &VerifyingKeyVar<MNT6_753, PairingVar>,
        pedersen_generators: &Vec<G1Var>,
    ) -> Result<Vec<UInt8<MNT4Fr>>, SynthesisError> {
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
        let serialized_bits = SerializeGadget::serialize_g1(cs.clone(), &hash)?;

        let serialized_bits = reverse_inner_byte_order(&serialized_bits[..]);

        // Convert to bytes.
        let mut bytes = Vec::new();

        for i in 0..serialized_bits.len() / 8 {
            bytes.push(UInt8::from_bits_le(&serialized_bits[i * 8..(i + 1) * 8]));
        }

        Ok(bytes)
    }
}
