use algebra::mnt4_753::{Fq, FqParameters, MNT4_753};
use algebra::mnt6_753::Fr as MNT6Fr;
use crypto_primitives::nizk::groth16::constraints::VerifyingKeyGadget;
use r1cs_core::SynthesisError;
use r1cs_std::bits::{boolean::Boolean, uint8::UInt8};
use r1cs_std::mnt4_753::{G1Gadget, PairingGadget};
use r1cs_std::ToBitsGadget;

use crate::gadgets::mnt6::{PedersenCommitmentGadget, YToBitGadget};
use crate::utils::{pad_point_bits, reverse_inner_byte_order};

/// This gadget is meant to calculate a commitment in-circuit for a verifying key of a SNARK in the
/// MNT4-753 curve. This means we can open this commitment inside of a circuit in the MNT6-753 curve
/// and we can use it to verify a SNARK proof inside that circuit.
/// We calculate it by first serializing the verifying key and feeding it to the Pedersen commitment
/// function, then we serialize the output and convert it to bytes. This provides an efficient way
/// of compressing the state and representing it across different curves.
pub struct VKCommitmentGadget;

impl VKCommitmentGadget {
    /// Calculates the verifying key commitment.
    pub fn evaluate<CS: r1cs_core::ConstraintSystem<MNT6Fr>>(
        mut cs: CS,
        vk: &VerifyingKeyGadget<MNT4_753, Fq, PairingGadget>,
        pedersen_generators: &Vec<G1Gadget>,
        sum_generator: &G1Gadget,
    ) -> Result<Vec<UInt8>, SynthesisError> {
        // Initialize Boolean vector.
        let mut bits: Vec<Boolean> = vec![];

        // Serialize the verifying key into bits.
        // Alpha G1
        let x_bits = vk.alpha_g1.x.to_bits(cs.ns(|| "x to bits: alpha g1"))?;
        let y_bit = YToBitGadget::y_to_bit_g1(cs.ns(|| "y to bit: alpha g1"), &vk.alpha_g1)?;
        bits.extend(pad_point_bits::<FqParameters>(x_bits, y_bit));
        // Beta G2
        let x_bits = vk.beta_g2.x.to_bits(cs.ns(|| "x to bits: beta g2"))?;
        let y_bit = YToBitGadget::y_to_bit_g2(cs.ns(|| "y to bit: beta g2"), &vk.beta_g2)?;
        bits.extend(pad_point_bits::<FqParameters>(x_bits, y_bit));
        // Gamma G2
        let x_bits = vk.gamma_g2.x.to_bits(cs.ns(|| "x to bits: gamma g2"))?;
        let y_bit = YToBitGadget::y_to_bit_g2(cs.ns(|| "y to bit: gamma g2"), &vk.gamma_g2)?;
        bits.extend(pad_point_bits::<FqParameters>(x_bits, y_bit));
        // Delta G2
        let x_bits = vk.delta_g2.x.to_bits(cs.ns(|| "x to bits: delta g2"))?;
        let y_bit = YToBitGadget::y_to_bit_g2(cs.ns(|| "y to bit: delta g2"), &vk.delta_g2)?;
        bits.extend(pad_point_bits::<FqParameters>(x_bits, y_bit));
        // Gamma ABC G1
        for i in 0..vk.gamma_abc_g1.len() {
            let point = &vk.gamma_abc_g1[i];
            let x_bits = point
                .x
                .to_bits(cs.ns(|| format!("x to bits: gamma abc g1: point {}", i)))?;
            let y_bit = YToBitGadget::y_to_bit_g1(
                cs.ns(|| format!("y to bit: gamma abc g1: point {}", i)),
                point,
            )?;
            bits.extend(pad_point_bits::<FqParameters>(x_bits, y_bit));
        }

        // Calculate the Pedersen commitment.
        let pedersen_commitment = PedersenCommitmentGadget::evaluate(
            cs.ns(|| "pedersen commitment"),
            &bits,
            pedersen_generators,
            &sum_generator,
        )?;

        // Serialize the Pedersen commitment.
        let x_bits = pedersen_commitment
            .x
            .to_bits(cs.ns(|| "x to bits: pedersen commitment"))?;
        let greatest_bit = YToBitGadget::y_to_bit_g1(
            cs.ns(|| "y to bit: pedersen commitment"),
            &pedersen_commitment,
        )?;
        let serialized_bits = pad_point_bits::<FqParameters>(x_bits, greatest_bit);
        let serialized_bits = reverse_inner_byte_order(&serialized_bits[..]);

        // Convert to bytes.
        let mut bytes = Vec::new();
        for i in 0..serialized_bits.len() / 8 {
            bytes.push(UInt8::from_bits_le(&serialized_bits[i * 8..(i + 1) * 8]));
        }

        Ok(bytes)
    }
}
