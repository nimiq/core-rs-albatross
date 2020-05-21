use algebra::mnt6_753::G2Projective;

use crate::constants::sum_generator_g1_mnt6;
use crate::primitives::{merkle_tree_construct, pedersen_commitment, pedersen_generators};
use crate::utils::{bytes_to_bits, serialize_g1_mnt6, serialize_g2_mnt6};

/// This gadget is meant to calculate the "state commitment" off-circuit, which is simply a commitment,
/// for a given block, of the block number concatenated with the root of a Merkle tree over the public
/// keys. We calculate it by first creating a Merkle tree from the public keys. Then we serialize the
/// block number and the Merkle tree root and feed it to the Pedersen commitment function. Lastly we
/// serialize the output and convert it to bytes. This provides an efficient way of compressing the
/// state and representing it across different curves.
pub fn state_commitment(
    block_number: u32,
    public_keys: Vec<G2Projective>,
    tree_size: usize,
) -> Vec<u8> {
    // Checking that the number of leaves is a power of two.
    assert!(((tree_size & (tree_size - 1)) == 0));

    // Checking that the number of public keys is a multiple of the number of leaves.
    assert_eq!(public_keys.len() % tree_size, 0);

    // Construct the Merkle tree over the public keys.
    let mut bytes: Vec<u8> = Vec::new();

    for i in 0..public_keys.len() {
        bytes.extend_from_slice(serialize_g2_mnt6(public_keys[i]).as_ref());
    }

    let bits = bytes_to_bits(&bytes);

    let mut inputs = Vec::new();

    for i in 0..tree_size {
        inputs.push(bits[i * tree_size..(i + 1) * tree_size].to_vec());
    }

    let root = merkle_tree_construct(inputs);

    // Serialize the block number and the Merkle tree root into bits.
    let mut bytes: Vec<u8> = vec![];

    bytes.extend_from_slice(&block_number.to_be_bytes());

    bytes.extend(&root);

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
