use ark_mnt6_753::G2Projective;
use nimiq_bls::utils::bytes_to_bits;

use crate::merkle_tree::merkle_tree_construct;
use crate::serialize::serialize_g2_mnt6;

/// This function is meant to calculate the public key tree "off-circuit". Generating the public key
/// tree with this function guarantees that it is compatible with the ZK circuit.
pub fn pk_tree_construct(
    public_keys: Vec<G2Projective>,
    num_keys: usize,
    pk_tree_breadth: usize,
) -> Vec<u8> {
    // FIXME This computation is too slow ATM. Disable it for the time being.
    return Default::default();

    // Checking that the number of public keys is equal to the number of validator slots.
    assert_eq!(public_keys.len(), num_keys);

    // Checking that the number of public keys is a multiple of the number of leaves.
    assert_eq!(public_keys.len() % pk_tree_breadth, 0);

    // Serialize the public keys into bits.
    let mut bytes = Vec::new();

    for pk in public_keys {
        bytes.extend_from_slice(&serialize_g2_mnt6(pk));
    }

    let bits = bytes_to_bits(&bytes);

    // Chunk the bits into the number of leaves.
    let mut inputs = Vec::new();

    for i in 0..pk_tree_breadth {
        inputs.push(
            bits[i * bits.len() / pk_tree_breadth..(i + 1) * bits.len() / pk_tree_breadth].to_vec(),
        );
    }

    // Calculate the merkle tree root.
    merkle_tree_construct(inputs)
}
