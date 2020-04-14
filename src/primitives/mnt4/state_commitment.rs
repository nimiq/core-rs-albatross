use algebra::mnt6_753::G2Projective;
use nimiq_bls::{PublicKey, Signature};

use crate::constants::sum_generator_g1_mnt6;
use crate::gadgets::bytes_to_bits;
use crate::primitives::mnt4::{pedersen_commitment, pedersen_generators};

/// This function is meant to calculate the "state commitment" off-circuit, which is simply a commitment,
/// for a given block, of the block number concatenated with the public_keys. We calculate it by first
/// serializing the block number and the public keys and feeding it to the Pedersen commitment
/// function, then we serialize the output and convert it to bytes. This provides an efficient way
/// of compressing the state and representing it across different curves.
pub fn state_commitment(block_number: u32, public_keys: Vec<G2Projective>) -> Vec<u8> {
    // Serialize the state into bits.
    let mut bytes: Vec<u8> = vec![];
    bytes.extend_from_slice(&block_number.to_be_bytes());
    for key in public_keys.iter() {
        let pk = PublicKey { public_key: *key };
        bytes.extend_from_slice(pk.compress().as_ref());
    }
    let bits = bytes_to_bits(&bytes);

    //Calculate the Pedersen generators and the sum generator. The formula used for the ceiling
    // division of x/y is (x+y-1)/y.
    let generators_needed = (bits.len() + 752 - 1) / 752;
    let generators = pedersen_generators(generators_needed);
    let sum_generator = sum_generator_g1_mnt6();

    // Calculate the Pedersen commitment.
    let pedersen_commitment = pedersen_commitment(generators, bits, sum_generator);

    // Serialize the Pedersen commitment.
    // TODO: This is a very ugly way of doing this...
    let sig = Signature {
        signature: pedersen_commitment,
    };
    Vec::from(sig.compress().as_ref())
}
