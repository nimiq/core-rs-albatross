use algebra::bls12_377::G2Projective;
use crypto_primitives::prf::Blake2sWithParameterBlock;
use nimiq_bls::PublicKey;

/// Calculates the Blake2s hash for the block from:
/// block number || public_keys.
pub fn evaluate_state_hash(block_number: u32, public_keys: &Vec<G2Projective>) -> Vec<u8> {
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
    blake2s.evaluate(bytes.as_ref())
}
