use algebra::bls12_377::G2Projective;
use byteorder::{ByteOrder, LittleEndian};
use crypto_primitives::prf::Blake2sWithParameterBlock;
use nimiq_bls::PublicKey;

/// This function is meant to calculate the "state hash" off-circuit, which is simply the Blake2s
/// hash, for a given block, of the block number concatenated with the public_keys. It is used as
/// input for all of the three zk-SNARK circuits.
pub fn evaluate_state_hash(block_number: u32, public_keys: &Vec<G2Projective>) -> Vec<u32> {
    // Create byte vector.
    let mut bytes: Vec<u8> = vec![];
    bytes.extend_from_slice(&block_number.to_be_bytes());
    for key in public_keys.iter() {
        let pk = PublicKey { public_key: *key };
        bytes.extend_from_slice(pk.compress().as_ref());
    }

    // Initialize Blake2s parameters.
    let blake2s = Blake2sWithParameterBlock {
        digest_length: 32,
        key_length: 0,
        fan_out: 1,
        depth: 1,
        leaf_length: 0,
        node_offset: 0,
        xof_digest_length: 0,
        node_depth: 0,
        inner_length: 0,
        salt: [0; 8],
        personalization: [0; 8],
    };

    // Calculate the hash.
    let hash = blake2s.evaluate(bytes.as_ref());

    let mut result = vec![];
    for i in 0..8 {
        let chunk = &hash[4 * i..4 * i + 4];
        result.push(LittleEndian::read_u32(chunk));
    }

    result
}
