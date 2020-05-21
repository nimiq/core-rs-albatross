use algebra::mnt6_753::MNT6_753;
use algebra_core::AffineCurve;
use groth16::VerifyingKey;

use crate::constants::sum_generator_g1_mnt6;
use crate::primitives::{pedersen_commitment, pedersen_generators};
use crate::utils::{bytes_to_bits, serialize_g1_mnt6, serialize_g2_mnt6};

/// This function is meant to calculate a commitment off-circuit for a verifying key of a SNARK in the
/// MNT6-753 curve. This means we can open this commitment inside of a circuit in the MNT4-753 curve
/// and we can use it to verify a SNARK proof inside that circuit.
/// We calculate it by first serializing the verifying key and feeding it to the Pedersen commitment
/// function, then we serialize the output and convert it to bytes. This provides an efficient way
/// of compressing the state and representing it across different curves.
pub fn vk_commitment(vk: VerifyingKey<MNT6_753>) -> Vec<u8> {
    // Serialize the verifying key into bits.
    let mut bytes: Vec<u8> = vec![];

    bytes.extend_from_slice(serialize_g1_mnt6(vk.alpha_g1.into_projective()).as_ref());

    bytes.extend_from_slice(serialize_g2_mnt6(vk.beta_g2.into_projective()).as_ref());

    bytes.extend_from_slice(serialize_g2_mnt6(vk.gamma_g2.into_projective()).as_ref());

    bytes.extend_from_slice(serialize_g2_mnt6(vk.delta_g2.into_projective()).as_ref());

    for i in 0..vk.gamma_abc_g1.len() {
        bytes.extend_from_slice(serialize_g1_mnt6(vk.gamma_abc_g1[i].into_projective()).as_ref());
    }

    let bits = bytes_to_bits(&bytes);

    //Calculate the Pedersen generators and the sum generator. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let generators_needed = (bits.len() + 752 - 1) / 752;

    let generators = pedersen_generators(generators_needed);

    let sum_generator = sum_generator_g1_mnt6();

    // Calculate the Pedersen commitment.
    let pedersen_commitment = pedersen_commitment(bits, generators, sum_generator);

    // Serialize the Pedersen commitment.
    let bytes = serialize_g1_mnt6(pedersen_commitment);

    Vec::from(bytes.as_ref())
}
