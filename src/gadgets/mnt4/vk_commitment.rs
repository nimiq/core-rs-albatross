use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::{Fq, MNT6_753};
use crypto_primitives::nizk::groth16::constraints::VerifyingKeyGadget;
use r1cs_core::SynthesisError;
use r1cs_std::bits::{boolean::Boolean, uint8::UInt8};
use r1cs_std::mnt6_753::{G1Gadget, PairingGadget};

use crate::gadgets::mnt4::{PedersenCommitmentGadget, SerializeGadget};
use crate::utils::reverse_inner_byte_order;

/// This gadget is meant to calculate a commitment in-circuit for a verifying key of a SNARK in the
/// MNT6-753 curve. This means we can open this commitment inside of a circuit in the MNT4-753 curve
/// and we can use it to verify a SNARK proof inside that circuit.
/// We calculate it by first serializing the verifying key and feeding it to the Pedersen commitment
/// function, then we serialize the output and convert it to bytes. This provides an efficient way
/// of compressing the state and representing it across different curves.
pub struct VKCommitmentGadget;

impl VKCommitmentGadget {
    /// Calculates the verifying key commitment.
    pub fn evaluate<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        vk: &VerifyingKeyGadget<MNT6_753, Fq, PairingGadget>,
        pedersen_generators: &Vec<G1Gadget>,
        sum_generator: &G1Gadget,
    ) -> Result<Vec<UInt8>, SynthesisError> {
        // Initialize Boolean vector.
        let mut bits: Vec<Boolean> = vec![];

        // Serialize the verifying key into bits.
        // Alpha G1
        bits.extend(SerializeGadget::serialize_g1(
            cs.ns(|| "serialize alpha g1"),
            &vk.alpha_g1,
        )?);
        // Beta G2
        bits.extend(SerializeGadget::serialize_g2(
            cs.ns(|| "serialize beta g2"),
            &vk.beta_g2,
        )?);
        // Gamma G2
        bits.extend(SerializeGadget::serialize_g2(
            cs.ns(|| "serialize gamma g2"),
            &vk.gamma_g2,
        )?);
        // Delta G2
        bits.extend(SerializeGadget::serialize_g2(
            cs.ns(|| "serialize delta g2"),
            &vk.delta_g2,
        )?);
        // Gamma ABC G1
        for i in 0..vk.gamma_abc_g1.len() {
            bits.extend(SerializeGadget::serialize_g1(
                cs.ns(|| format!("serialize gamma abc g1: point {}", i)),
                &vk.gamma_abc_g1[i],
            )?);
        }

        // Calculate the Pedersen commitment.
        let pedersen_commitment = PedersenCommitmentGadget::evaluate(
            cs.ns(|| "pedersen commitment"),
            &bits,
            pedersen_generators,
            &sum_generator,
        )?;

        // Serialize the Pedersen commitment.
        let serialized_bits = SerializeGadget::serialize_g1(
            cs.ns(|| "serialize pedersen commitment"),
            &pedersen_commitment,
        )?;
        let serialized_bits = reverse_inner_byte_order(&serialized_bits[..]);

        // Convert to bytes.
        let mut bytes = Vec::new();
        for i in 0..serialized_bits.len() / 8 {
            bytes.push(UInt8::from_bits_le(&serialized_bits[i * 8..(i + 1) * 8]));
        }

        Ok(bytes)
    }
}
