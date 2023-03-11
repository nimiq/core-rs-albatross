use ark_crypto_primitives::crh::pedersen::{Parameters, Window};
use ark_ec::{AffineRepr, CurveGroup, Group};
use ark_ff::{One, PrimeField, ToConstraintField};
use ark_mnt6_753::{Fq, G1Affine, G1Projective};

use nimiq_hash::blake2s::Blake2sWithParameterBlock;

use crate::rand_gen::generate_random_seed;

pub const POINT_CAPACITY: usize = Fq::MODULUS_BIT_SIZE as usize - 1; // 752

#[derive(Clone)]
pub struct PedersenParameters<C: CurveGroup> {
    pub parameters: Parameters<C>,
    pub blinding_factor: C,
}

impl<C: CurveGroup> PedersenParameters<C> {
    pub fn sub_window<W: Window>(&self) -> Self {
        PedersenParameters {
            parameters: Parameters {
                generators: self
                    .parameters
                    .generators
                    .iter()
                    .take(W::NUM_WINDOWS)
                    .cloned()
                    .collect(),
            },
            blinding_factor: self.blinding_factor,
        }
    }
}

/// This is the function for creating generators in the G1 group for the MNT6-753 curve. These
/// generators are meant to be used for the Pedersen hash function.
fn pedersen_generators(number: usize) -> Vec<G1Projective> {
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
        let mut blake2x = Blake2sWithParameterBlock::new_blake2x(i, 0xffff);
        // This needs to be set to an unique value, since we want a different random stream for
        // each generator series that we create. So we take a random u64 and convert it to bytes.
        // The random u64 came from random.org.
        blake2x.personalization = 2813876015388210123_u64.to_be_bytes();

        let mut result = blake2x.evaluate(&seed);

        bytes.append(&mut result);
    }

    // Initialize the vector that will contain the generators.
    let mut generators = Vec::new();

    // Generating the generators.
    for n in 0..number {
        // This converts the hash output into an x-coordinate and a y-coordinate for an elliptic curve
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

        // The y-coordinate is at the most significant bit (interpreted as little endian). We convert it to a boolean.
        let bytes = &bytes[n * 96..n * 96 + 96];
        let y_bit = (bytes[bytes.len() - 1] >> 7) & 1 == 1;

        // Because of the previous explanation, we need to remove the whole last two bytes.
        let max_size = ((Fq::MODULUS_BIT_SIZE - 1) / 8) as usize;
        let x_coordinates = ToConstraintField::to_field_elements(&bytes[..max_size]).unwrap();
        assert_eq!(x_coordinates.len(), 1);
        let mut x_coordinate = x_coordinates[0];

        // This implements the try-and-increment method of converting an integer to an elliptic curve point.
        // See https://eprint.iacr.org/2009/226.pdf for more details.
        loop {
            let point = G1Affine::get_point_from_x_unchecked(x_coordinate, y_bit);

            if let Some(g1) = point {
                let scaled_g1 = g1.mul_by_cofactor_to_group();

                generators.push(scaled_g1);

                break;
            }

            x_coordinate += &Fq::one();
        }
    }

    generators
}

pub fn pedersen_generator_powers<W: Window>() -> PedersenParameters<G1Projective> {
    let generators = pedersen_generators(W::NUM_WINDOWS + 1);
    let blinding_factor = generators[0];

    // Pre-calculate POINT_CAPACITY powers of the generators.
    let mut generator_powers = Vec::with_capacity(W::NUM_WINDOWS);
    for generator in generators.into_iter().skip(1) {
        let mut powers = Vec::with_capacity(W::WINDOW_SIZE);
        let mut power = generator;
        powers.push(power);
        for _ in 1..W::WINDOW_SIZE {
            power.double_in_place();
            powers.push(power);
        }
        generator_powers.push(powers);
    }
    PedersenParameters {
        parameters: Parameters {
            generators: generator_powers,
        },
        blinding_factor,
    }
}
