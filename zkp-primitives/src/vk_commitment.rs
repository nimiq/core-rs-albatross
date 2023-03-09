use ark_ec::AffineRepr;
use ark_groth16::VerifyingKey;
use ark_mnt6_753::MNT6_753;

use crate::{pedersen::default_pedersen_hash, serialize_g1_mnt6, serialize_g2_mnt6};

/// This function is meant to calculate a commitment off-circuit for a verifying key of a SNARK in the
/// MNT6-753 curve. This means we can open this commitment inside of a circuit in the MNT4-753 curve
/// and we can use it to verify a SNARK proof inside that circuit.
/// We calculate it by first serializing the verifying key and feeding it to the Pedersen hash
/// function, then we serialize the output and convert it to bits. This provides an efficient way
/// of compressing the state and representing it across different curves.
pub fn vk_commitment(vk: VerifyingKey<MNT6_753>) -> [u8; 95] {
    // Serialize the verifying key into bits.
    let mut bytes: Vec<u8> = vec![];

    bytes.extend_from_slice(serialize_g1_mnt6(&vk.alpha_g1.into_group()).as_ref());

    bytes.extend_from_slice(serialize_g2_mnt6(&vk.beta_g2.into_group()).as_ref());

    bytes.extend_from_slice(serialize_g2_mnt6(&vk.gamma_g2.into_group()).as_ref());

    bytes.extend_from_slice(serialize_g2_mnt6(&vk.delta_g2.into_group()).as_ref());

    for i in 0..vk.gamma_abc_g1.len() {
        bytes.extend_from_slice(serialize_g1_mnt6(&vk.gamma_abc_g1[i].into_group()).as_ref());
    }

    // Calculate the Pedersen hash.
    let hash = default_pedersen_hash(&bytes);

    // Serialize the Pedersen commitment.
    serialize_g1_mnt6(&hash)
}
