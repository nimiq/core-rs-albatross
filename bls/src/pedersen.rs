use ark_crypto_primitives::prf::Blake2sWithParameterBlock;
use ark_ec::group::Group;
use ark_ff::{FpParameters, One, PrimeField};
use ark_mnt6_753::{Fq, FqParameters, G1Affine, G1Projective};
use blake2_rfc::blake2s::Blake2s;

use crate::rand_gen::generate_random_seed;
use crate::utils::big_int_from_bytes_be;

const POINT_CAPACITY: usize = FqParameters::CAPACITY as usize;

/// This is the function for creating generators in the G1 group for the MNT6-753 curve. These
/// generators are meant to be used for the Pedersen hash function.
pub fn pedersen_generators(number: usize) -> Vec<G1Projective> {
    // This gets a verifiably random seed. Whenever we use this seed we need to set the personalization
    // field on the Blake2X to an unique value. See below.
    let seed = generate_random_seed();

    // This extends the seed using the Blake2X algorithm.
    // See https://blake2.net/blake2x.pdf for more details.
    // We need 96 bytes of output for each generator that we are going to create.
    // The number of rounds is calculated so that we get 96 bytes per generator needed.
    let mut bytes = vec![];

    let number_rounds = number * 3;

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
            // This needs to be set to an unique value, since we want a different random stream for
            // each generator series that we create. So we take a random u64 and convert it to bytes.
            // The random u64 came from random.org.
            personalization: 2813876015388210123_u64.to_be_bytes(),
        };

        let mut state = Blake2s::with_parameter_block(&blake2x.parameters());

        state.update(&seed);

        let mut result = state.finalize().as_bytes().to_vec();

        bytes.append(&mut result);
    }

    // Initialize the vector that will contain the generators.
    let mut generators = Vec::new();

    // Generating the generators.
    for i in 0..number {
        // This converts the hash output into a x-coordinate and a y-coordinate for an elliptic curve
        // point. At this time, it is not guaranteed to be a valid point. A quirk of this code is that
        // we need to set the most significant 16 bits to zero. The reason for this is that the field for
        // the MNT6-753 curve is not exactly 753 bits, it is a bit smaller. This means that if we try
        // to create a field element from 753 random bits, we may get an invalid value back (in this
        // case it is just all zeros). There are two options to deal with this:
        // 1) To create finite field elements, using 753 random bits, in a loop until a valid one
        //    is created.
        // 2) Use only 752 random bits to create a finite field element. This will guaranteedly
        //    produce a valid element on the first try, but will reduce the entropy of the EC
        //    point generation by one bit.
        // We chose the second one because we believe the entropy reduction is not significant enough.
        // Since we have 768 bits per generator but only need 752 bits, we set the first 16 bits (768-752=16)
        // to zero.
        // The y-coordinate is at the first bit. We convert it to a boolean.
        let y_coordinate = (bytes[96 * i] >> 7) & 1 == 1;

        // In order to easily read the BigInt from the bytes, we use the first 16 bits as padding.
        // However, because of the previous explanation, we need to nullify the whole first two bytes.
        bytes[96 * i] = 0;

        bytes[96 * i + 1] = 0;

        let mut x_coordinate =
            Fq::from_repr(big_int_from_bytes_be(&mut &bytes[96 * i..96 * (i + 1)])).unwrap();

        // This implements the try-and-increment method of converting an integer to an elliptic curve point.
        // See https://eprint.iacr.org/2009/226.pdf for more details.
        loop {
            let point = G1Affine::get_point_from_x(x_coordinate, y_coordinate);

            if let Some(g1) = point {
                let scaled_g1 = g1.scale_by_cofactor();

                generators.push(scaled_g1);

                break;
            }

            x_coordinate += &Fq::one();
        }
    }

    generators
}

/// Calculates the Pedersen hash. Given a vector of bits b_i we divide the vector into chunks
/// of 752 bits and convert them into scalars like so:
/// s = b_0 * 2^0 + b_1 * 2^1 + ... + b_750 * 2^750 + b_751 * 2^751
/// We then calculate the commitment like so:
/// H = G_0 + s_1 * G_1 + ... + s_n * G_n
/// where G_0 is a sum generator that is used to avoid that the sum starts at zero (which is
/// problematic because the ZK circuits used in nano-sync can't handle addition with zero).
pub fn pedersen_hash(input: Vec<bool>, generators: Vec<G1Projective>) -> G1Projective {
    // Check that the input can be stored using the available generators.
    assert!((generators.len() - 1) * POINT_CAPACITY >= input.len());

    // Calculate the rounds that are necessary to process the input.
    let normal_rounds = input.len() / POINT_CAPACITY;

    let bits_last_round = input.len() % POINT_CAPACITY;

    // Initialize the sum to the first generator.
    let mut result = generators[0];

    let mut power = generators[1];

    // Start calculating the Pedersen hash.
    for i in 0..normal_rounds {
        // We multiply each generator by the corresponding scalar formed from 752 bits of input. This
        // is the double-and-add method for EC point multiplication.
        // (https://en.wikipedia.org/wiki/Elliptic_curve_point_multiplication#Double-and-add)
        for k in 0..POINT_CAPACITY {
            if input[i * POINT_CAPACITY + k] {
                result += power;
            }
            power.double_in_place();
        }
        power = generators[i + 2];
    }

    // Begin the final point multiplication. For this one we don't use all 752 bits.
    for k in 0..bits_last_round {
        if input[normal_rounds * POINT_CAPACITY + k] {
            result += power;
        }
        power.double_in_place();
    }

    result
}
