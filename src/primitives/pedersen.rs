use algebra::mnt6_753::{Fq, G1Affine, G1Projective};
use algebra_core::{One, PrimeField};
use blake2_rfc::blake2s::Blake2s;
use crypto_primitives::prf::Blake2sWithParameterBlock;
use nimiq_bls::big_int_from_bytes_be;

use crate::rand_gen::generate_random_seed;

/// The size of the input to the Pedersen hash, in bytes. It must be know ahead of time in order to
/// create the vector of generators. We need one generator per bit of input (8 per byte).
pub const INPUT_SIZE: usize = 32;

/// This is the function for generating the Pedersen hash generators for our specific instance. We
/// need one generator per each bit of input.
pub fn setup_pedersen() -> Vec<G1Projective> {
    // This gets a verifiably random seed.
    let seed = generate_random_seed();

    // This extends the seed using the Blake2X algorithm.
    // See https://blake2.net/blake2x.pdf for more details.
    // We need 96 bytes of output for each generator that we are going to create.
    // The number of rounds is calculated so that we get 48 bytes per generator needed. We use the
    // following formula for the ceiling division: |x/y| = (x+y-1)/y
    let mut bytes = vec![];
    let number_rounds = (INPUT_SIZE * 8 * 96 + 32 - 1) / 32;
    for i in 0..number_rounds {
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
            salt: [1; 8],
            personalization: [1; 8], // We set this to be different from the one used to generate the sum generators in constants.rs
        };
        let mut state = Blake2s::with_parameter_block(&blake2x.parameters());
        state.update(&seed);
        let mut result = state.finalize().as_bytes().to_vec();
        bytes.append(&mut result);
    }

    // Initialize the vector that will contain the generators.
    let mut generators = Vec::new();

    // Generating the generators.
    for i in 0..INPUT_SIZE * 8 {
        // This converts the hash output into a x-coordinate and a y-coordinate for an elliptic curve point. At this time, it is not guaranteed to be a valid point.
        // A quirk of this code is that we need to set the most significant bit to zero. The reason for this is that the field for the BLS12-377 curve is not exactly 377 bits, it is a bit smaller.
        // This means that if we try to create a field element from 377 random bits, we may get an invalid value back (in this case it is just all zeros). There are two options to deal with this:
        // 1) To create finite field elements, using 377 random bits, in a loop until a valid one is created.
        // 2) Use only 376 random bits to create a finite field element. This will guaranteedly produce a valid element on the first try, but will reduce the entropy of the EC point generation by one bit.
        // We chose the second one because we believe the entropy reduction is not significant enough.
        // The y-coordinate is at first bit.
        let y_coordinate = (bytes[96 * i] >> 7) & 1 == 1;

        // In order to easily read the BigInt from the bytes, we use the first 7 bits as padding.
        // However, because of the previous explanation, we also need to set the 8th bit to 0.
        // Thus, we can nullify the whole first byte.
        bytes[96 * i] = 0;
        bytes[96 * i + 1] = 0;
        let mut x_coordinate =
            Fq::from_repr(big_int_from_bytes_be(&mut &bytes[96 * i..96 * (i + 1)]));

        // This implements the try-and-increment method of converting an integer to an elliptic curve point.
        // See https://eprint.iacr.org/2009/226.pdf for more details.
        loop {
            let point = G1Affine::get_point_from_x(x_coordinate, y_coordinate);
            if point.is_some() {
                let g1 = point.unwrap().scale_by_cofactor();
                generators.push(g1);
                break;
            }
            x_coordinate += &Fq::one();
        }
    }

    generators
}

/// Calculates the Pedersen hash. Given a vector of generators G_i and a vector of bits b_i, the
/// hash is calculated like so:
/// H = b_1*G_1 + b_2*G_2 + ... + b_n*G_n
pub fn evaluate_pedersen(
    generators: Vec<G1Projective>,
    input: Vec<u8>,
    sum_generator: G1Projective,
) -> G1Projective {
    assert_eq!(generators.len(), input.len() * 8);

    // Convert the input bytes to bits.
    let mut bits = vec![];
    for i in 0..input.len() {
        let byte = input[i];
        for j in 0..8 {
            bits.push((byte >> j) & 1 == 1);
        }
    }

    // Initialize the sum to the generator. Normally it would be zero, but this is necessary because
    // of some complications in the PedersenHashGadget.
    let mut result = sum_generator;

    // Calculate the hash by adding a generator whenever the corresponding bit is set to 1.
    for i in 0..bits.len() {
        if bits[i] {
            result += &generators[i];
        }
    }

    result
}
