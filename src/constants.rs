//! This module contains several constants that are used throughout the library. They can be changed
//! easily here, without needing to change them in several places in the code.

use algebra::mnt6_753::{Fq, Fq3, G1Affine, G1Projective, G2Affine, G2Projective};
use algebra::PrimeField;
use algebra_core::One;
use blake2_rfc::blake2s::Blake2s;
use crypto_primitives::prf::Blake2sWithParameterBlock;
use nimiq_bls::big_int_from_bytes_be;

use crate::rand_gen::generate_random_seed;

/// This is the length of one epoch in Albatross. Basically, the difference in the block numbers of
/// two consecutive macro blocks.
pub const EPOCH_LENGTH: u32 = 128;

/// This is the number of validator slots in Albatross.
/// VALIDATOR_SLOTS = MIN_SIGNERS + MAX_NON_SIGNERS
pub const VALIDATOR_SLOTS: usize = 4;

/// This is the minimum number of validator slots that must sign a macro block in order to be valid.
/// MIN_SIGNERS = ceiling( VALIDATOR_SLOTS * 2/3 )
/// The formula used for the ceiling division of x/y is (x+y-1)/y.
pub const MIN_SIGNERS: usize = (VALIDATOR_SLOTS * 2 + 3 - 1) / 3;

/// This the maximum number of validator slots that can NOT sign a macro block and it still being valid.
/// MAX_NON_SIGNERS = floor( VALIDATOR_SLOTS/3 )
pub const MAX_NON_SIGNERS: usize = VALIDATOR_SLOTS / 3;

/// Creates a generator point for the G1 group in the MNT6-753 curve, from a verifiably random seed.
/// The generator is mainly used as a "buffer" when adding elliptic curves. The buffer is necessary
/// sometimes because the addition formula for elliptic curves in the gadgets is incomplete and can't
/// handle the identity element (aka zero).
pub fn sum_generator_g1_mnt6() -> G1Projective {
    // This gets a verifiably random seed.
    let seed = generate_random_seed();

    // This extends the seed using the Blake2X algorithm.
    // See https://blake2.net/blake2x.pdf for more details.
    // We only need 95 bytes of output, but for simplicity output 96 bytes.
    let mut bytes = vec![];
    for i in 0..3 {
        let blake2x = Blake2sWithParameterBlock {
            digest_length: 32,
            key_length: 0,
            fan_out: 0,
            depth: 0,
            leaf_length: 32,
            node_offset: i as u32,
            xof_digest_length: 65535,
            node_depth: 0,
            inner_length: 32,
            salt: [0; 8],
            personalization: [0; 8],
        };
        let mut state = Blake2s::with_parameter_block(&blake2x.parameters());
        state.update(&seed);
        let mut result = state.finalize().as_bytes().to_vec();
        bytes.append(&mut result);
    }

    // This converts the hash output into a x-coordinate and a y-coordinate for an elliptic curve point. At this time, it is not guaranteed to be a valid point.
    // A quirk of this code is that we need to set the most significant bit to zero. The reason for this is that the field for the MNT6-753 curve is not exactly 753 bits, it is a bit smaller.
    // This means that if we try to create a field element from 753 random bits, we may get an invalid value back (in this case it is just all zeros). There are two options to deal with this:
    // 1) To create finite field elements, using 753 random bits, in a loop until a valid one is created.
    // 2) Use only 752 random bits to create a finite field element. This will guaranteedly produce a valid element on the first try, but will reduce the entropy of the EC point generation by one bit.
    // We chose the second one because we believe the entropy reduction is not significant enough.
    // The y-coordinate is at first bit.
    let y_coordinate = (bytes[0] >> 7) & 1 == 1;

    // In order to easily read the BigInt from the bytes, we use the first 7 bits as padding.
    // However, because of the previous explanation, we also need to set the 8th bit to 0.
    // Thus, we can nullify the whole first byte.
    bytes[0] = 0;
    bytes[1] = 0;
    let mut x_coordinate = Fq::from_repr(big_int_from_bytes_be(&mut &bytes[..96]));

    // This implements the try-and-increment method of converting an integer to an elliptic curve point.
    // See https://eprint.iacr.org/2009/226.pdf for more details.
    loop {
        let point = G1Affine::get_point_from_x(x_coordinate, y_coordinate);
        if point.is_some() {
            let g1 = point.unwrap().scale_by_cofactor();
            return g1;
        }
        x_coordinate += &Fq::one();
    }
}

/// Creates a generator point for the G2 group in the MNT6-753 curve, from a verifiably random seed.
/// The generator is mainly used as a "buffer" when adding elliptic curves. The buffer is necessary
/// sometimes because the addition formula for elliptic curves in the gadgets is incomplete and can't
/// handle the identity element (aka zero).
pub fn sum_generator_g2_mnt6() -> G2Projective {
    // This gets a verifiably random seed.
    let seed = generate_random_seed();

    // This extends the seed using the Blake2X algorithm.
    // See https://blake2.net/blake2x.pdf for more details.
    // We only need 285 bytes of output, but for simplicity output 288 bytes.
    let mut bytes = vec![];
    for i in 0..9 {
        let blake2x = Blake2sWithParameterBlock {
            digest_length: 32,
            key_length: 0,
            fan_out: 0,
            depth: 0,
            leaf_length: 32,
            node_offset: i as u32,
            xof_digest_length: 65535,
            node_depth: 0,
            inner_length: 32,
            salt: [0; 8],
            personalization: [0; 8],
        };
        let mut state = Blake2s::with_parameter_block(&blake2x.parameters());
        state.update(&seed);
        let mut result = state.finalize().as_bytes().to_vec();
        bytes.append(&mut result);
    }

    // This converts the hash output into a x-coordinate and a y-coordinate for an elliptic curve point. At this time, it is not guaranteed to be a valid point.
    // A quirk of this code is that we need to set the most significant bit to zero. The reason for this is that the field for the MNT6-753 curve is not exactly 753 bits, it is a bit smaller.
    // This means that if we try to create a field element from 753 random bits, we may get an invalid value back (in this case it is just all zeros). There are two options to deal with this:
    // 1) To create finite field elements, using 753 random bits, in a loop until a valid one is created.
    // 2) Use only 752 random bits to create a finite field element. This will guaranteedly produce a valid element on the first try, but will reduce the entropy of the EC point generation by one bit.
    // We chose the second one because we believe the entropy reduction is not significant enough.
    // The y-coordinate is at first bit.
    let y_coordinate = (bytes[0] >> 7) & 1 == 1;

    // In order to easily read the BigInt from the bytes, we use the first 7 bits as padding.
    // However, because of the previous explanation, we also need to set the 8th bit to 0.
    // Thus, we can nullify the whole first byte.
    bytes[0] = 0;
    bytes[1] = 0;
    bytes[96] = 0;
    bytes[97] = 0;
    bytes[192] = 0;
    bytes[193] = 0;
    let c0 = Fq::from_repr(big_int_from_bytes_be(&mut &bytes[..96]));
    let c1 = Fq::from_repr(big_int_from_bytes_be(&mut &bytes[96..192]));
    let c2 = Fq::from_repr(big_int_from_bytes_be(&mut &bytes[192..288]));
    let mut x_coordinate = Fq3::new(c0, c1, c2);

    // This implements the try-and-increment method of converting an integer to an elliptic curve point.
    // See https://eprint.iacr.org/2009/226.pdf for more details.
    loop {
        let point = G2Affine::get_point_from_x(x_coordinate, y_coordinate);
        if point.is_some() {
            let g2 = point.unwrap().scale_by_cofactor();
            return g2;
        }
        x_coordinate += &Fq3::one();
    }
}
