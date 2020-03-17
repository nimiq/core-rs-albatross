use algebra::bls12_377::{FqParameters, G2Affine, G2Projective};
use algebra::sw6::Fr as SW6Fr;
use algebra_core::ProjectiveCurve;
use crypto_primitives::prf::blake2s::Blake2s;
use crypto_primitives::prf::Blake2sWithParameterBlock;
use crypto_primitives::PRF;
use r1cs_core::SynthesisError;
use r1cs_std::bits::{boolean::Boolean, uint32::UInt32};
use r1cs_std::bls12_377::G2Gadget;
use r1cs_std::{ToBitsGadget, ToBytesGadget};

use crate::gadgets::{pad_point_bits, reverse_inner_byte_order, YToBitGadget};

/// Calculates the Blake2s hash for the block from:
/// block number || public_keys.
pub fn evaluate_state_hash(block_number: u32, public_keys: &Vec<G2Projective>) -> Vec<u8> {
    // Initialize Boolean vector.
    let mut bytes: Vec<u8> = vec![];

    // The block number comes in little endian all the way.
    // So, a reverse will put it into big endian.
    let mut block_number_be = block_number.to_be();
    bytes.append(&mut block_number_be);

    // Convert each public key to bytes and append it.
    for key in public_keys.iter() {
        let mut key_bytes = key.to_bytes()?;
        for byte in key_bytes {
            bytes.push(byte.get_value().unwrap());
        }
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
