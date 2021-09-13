use ark_mnt6_753::G2Projective;
use rayon::prelude::*;

use nimiq_bls::utils::bytes_to_bits;
use nimiq_primitives::policy::SLOTS;

use crate::merkle_tree::merkle_tree_construct;
use crate::serialize::serialize_g2_mnt6;

/// This is the depth of the PKTree circuit.
pub const PK_TREE_DEPTH: usize = 5;

/// This is the number of leaves in the PKTree circuit.
pub const PK_TREE_BREADTH: usize = 2_usize.pow(PK_TREE_DEPTH as u32);

/// This function is meant to calculate the public key tree "off-circuit". Generating the public key
/// tree with this function guarantees that it is compatible with the ZK circuit.
pub fn pk_tree_construct(public_keys: Vec<G2Projective>) -> Vec<u8> {
    // FIXME This computation is too slow ATM. Disable it for the time being.
    return Default::default();

    // Checking that the number of public keys is equal to the number of validator slots.
    assert_eq!(public_keys.len(), SLOTS as usize);

    // Checking that the number of public keys is a multiple of the number of leaves.
    assert_eq!(public_keys.len() % PK_TREE_BREADTH, 0);

    // Serialize the public keys into bits.
    let bits: Vec<bool> = public_keys
        .par_iter()
        .map(|pk| bytes_to_bits(&serialize_g2_mnt6(pk)))
        .flatten()
        .collect();

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
