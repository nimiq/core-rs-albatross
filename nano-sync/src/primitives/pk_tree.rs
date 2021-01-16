use ark_mnt6_753::G2Projective;

use crate::constants::{PK_TREE_BREADTH, VALIDATOR_SLOTS};
use crate::primitives::{merkle_tree_construct, serialize_g2_mnt6};
use crate::utils::bytes_to_bits;

/// This function is meant to calculate the public key tree "off-circuit". Generating the public key
/// tree with this function guarantees that it is compatible with the ZK circuit.
pub fn pk_tree_construct(public_keys: Vec<G2Projective>) -> Vec<u8> {
    // Checking that the number of public keys is equal to the number of validator slots.
    assert_eq!(public_keys.len(), VALIDATOR_SLOTS);

    // Checking that the number of public keys is a multiple of the number of leaves.
    assert_eq!(public_keys.len() % PK_TREE_BREADTH, 0);

    // Serialize the public keys into bits.
    let mut bytes = Vec::new();

    for i in 0..public_keys.len() {
        bytes.extend_from_slice(&serialize_g2_mnt6(public_keys[i]));
    }

    let bits = bytes_to_bits(&bytes);

    // Chunk the bits into the number of leaves.
    let mut inputs = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        inputs.push(
            bits[i * bits.len() / PK_TREE_BREADTH..(i + 1) * bits.len() / PK_TREE_BREADTH].to_vec(),
        );
    }

    // Calculate the merkle tree root.
    merkle_tree_construct(inputs)
}
