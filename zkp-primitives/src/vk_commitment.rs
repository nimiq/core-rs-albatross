use ark_ec::pairing::Pairing;
use ark_groth16::VerifyingKey;
use ark_serialize::CanonicalSerialize;

use crate::{pedersen::default_pedersen_hash, serialize_g1_mnt6};

/// This function is meant to calculate a commitment off-circuit for a verifying key of a SNARK in the
/// MNT6-753 curve. This means we can open this commitment inside of a circuit in the MNT4-753 curve
/// and we can use it to verify a SNARK proof inside that circuit.
/// We calculate it by first serializing the verifying key and feeding it to the Pedersen hash
/// function, then we serialize the output and convert it to bits. This provides an efficient way
/// of compressing the state and representing it across different curves.
pub fn vk_commitment<E: Pairing>(vk: VerifyingKey<E>) -> [u8; 95] {
    // Serialize the verifying key into bits.
    let mut serialized_vk = vec![];
    vk.serialize_compressed(&mut serialized_vk).unwrap();

    // Calculate the Pedersen hash.
    let hash = default_pedersen_hash(&serialized_vk);

    // Serialize the Pedersen commitment.
    serialize_g1_mnt6(&hash)
}

/// Combines multiple commitments into one.
pub fn vks_commitment(commitments: &[[u8; 95]]) -> [u8; 95] {
    let mut bytes: Vec<u8> = vec![];
    for commitment in commitments.iter() {
        bytes.extend(commitment);
    }

    // Calculate the Pedersen hash.
    let hash = default_pedersen_hash(&bytes);

    // Serialize the Pedersen commitment.
    serialize_g1_mnt6(&hash)
}
