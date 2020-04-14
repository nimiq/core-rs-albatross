use algebra::mnt6_753::{Fq, G1Affine, G1Projective};
use algebra_core::{Group, One, PrimeField};
use blake2_rfc::blake2s::Blake2s;
use crypto_primitives::prf::Blake2sWithParameterBlock;
use nimiq_bls::big_int_from_bytes_be;

use crate::rand_gen::generate_random_seed;

/// This is the function for creating generators in the G1 group for the MNT4-753 curve. These
/// generators are meant to be use for the Pedersen hash and the Pedersen commitment functions.
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
            personalization: [0; 8], // This needs to be set to an unique value. See above.
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
/// H = G_0 + b_1 * G_1 + b_2 * G_2 + ... + b_n * G_n
/// where G_0 is a sum generator that is used to avoid that the sum starts at zero (which is
/// problematic because the circuit can't handle addition with zero).
/// The Pedersen hash guarantees that the exponent of the resulting point H is not known, the same
/// can't be said of the Pedersen commitment. Also, note that the Pedersen hash is collision-resistant
/// but it is not pseudo-random.
pub fn pedersen_hash(
    generators: Vec<G1Projective>,
    input: Vec<bool>,
    sum_generator: G1Projective,
) -> G1Projective {
    // Verify that we have enough generators for the input bits.
    assert!(generators.len() >= input.len());

    // Initialize the sum to the generator. Normally it would be zero, but this is necessary because
    // of some complications in the PedersenHashGadget.
    let mut result = sum_generator;

    // Calculate the hash by adding a generator whenever the corresponding bit is set to 1.
    for i in 0..input.len() {
        if input[i] {
            result += &generators[i];
        }
    }

    result
}

/// Calculates the Pedersen commitment. Given a vector of bits b_i we divide the vector into chunks
/// of 752 bits and convert them into scalars like so:
/// s = b_0 * 2^0 + b_1 * 2^1 + ... + b_750 * 2^750 + b_751 * 2^751
/// We then calculate the commitment like so:
/// C = G_0 + s_1 * G_1 + ... + s_n * G_n
/// where G_0 is a sum generator that is used to avoid that the sum starts at zero (which is
/// problematic because the circuit can't handle addition with zero).
/// The Pedersen commitment takes the same time/constraints to calculate as the Pedersen hash but,
/// since it requires fewer generators, it is faster to setup.
pub fn pedersen_commitment(
    generators: Vec<G1Projective>,
    input: Vec<bool>,
    sum_generator: G1Projective,
) -> G1Projective {
    // This is simply the number of bits that each generator can store.
    let capacity = 752;

    // Check that the input can be stored using the available generators.
    assert!(generators.len() * capacity >= input.len());

    // Calculate the rounds that are necessary to process the input.
    let normal_rounds = input.len() / capacity;
    let bits_last_round = input.len() % capacity;

    // Initialize the sum to the generator. Normally it would be zero, but this is necessary because
    // of some complications in the PedersenHashGadget.
    let mut result = sum_generator;
    let mut power = generators[0];

    // Start calculating the Pedersen commitment.
    for i in 0..normal_rounds {
        // We multiply each generator by the corresponding scalar formed from 752 bits of input. This
        // is the double-and-add method for EC point multiplication.
        // (https://en.wikipedia.org/wiki/Elliptic_curve_point_multiplication#Double-and-add)
        for k in 0..capacity {
            if input[i * capacity + k] {
                result += power;
            }
            power.double_in_place();
        }
        power = generators[i + 1];
    }

    // Begin the final point multiplication. For this one we don't use all 752 bits.
    for k in 0..bits_last_round {
        if input[normal_rounds * capacity + k] {
            result += power;
        }
        power.double_in_place();
    }

    result
}
