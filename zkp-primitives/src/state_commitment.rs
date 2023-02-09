use ark_mnt6_753::G2Projective;

use nimiq_bls::pedersen::pedersen_hash;
use nimiq_bls::utils::bytes_to_bits_le;

use crate::pedersen_generator_powers::PEDERSEN_GENERATORS;
use crate::{pk_tree_construct, serialize_g1_mnt6};

/// This gadget is meant to calculate the "state commitment" off-circuit, which is simply a commitment,
/// for a given block, of the block number concatenated with the header hash concatenated with the
/// root of a Merkle tree over the public keys.
/// We calculate it by first creating a Merkle tree from the public keys. Then we serialize the
/// block number, the header hash and the Merkle tree root and feed it to the Pedersen hash function.
/// Lastly we serialize the output and convert it to bytes. This provides an efficient way of
/// compressing the state and representing it across different curves.
pub fn state_commitment(
    block_number: u32,
    header_hash: [u8; 32],
    public_keys: Vec<G2Projective>,
) -> Vec<u8> {
    // Construct the Merkle tree over the public keys.
    let root = pk_tree_construct(public_keys);

    // Serialize the block number, header hash and the Merkle tree root into bits.
    let mut bytes: Vec<u8> = vec![];

    bytes.extend_from_slice(&block_number.to_be_bytes());

    bytes.extend_from_slice(&header_hash);

    bytes.extend(&root);

    let bits = bytes_to_bits_le(&bytes);

    // Calculate the Pedersen hash.
    let hash = pedersen_hash(bits, &PEDERSEN_GENERATORS);

    // Serialize the Pedersen hash.
    let bytes = serialize_g1_mnt6(&hash);

    Vec::from(bytes.as_ref())
}
