use ark_crypto_primitives::crh::pedersen::{Parameters, Window};
use ark_ec::{pairing::Pairing, CurveGroup, Group};
use ark_ff::PrimeField;
use ark_mnt6_753::Fq;
use ark_std::UniformRand;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

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

/// This is the function for creating generators in the G1 group. These
/// generators are meant to be used for the Pedersen hash function.
fn pedersen_generators<P: Pairing>(number: usize, personalization: u64) -> Vec<P::G1> {
    // This gets a verifiably random seed. We pass in the personalization value to get unique results.
    let seed = generate_random_seed(personalization);
    let mut rng = ChaCha20Rng::from_seed(seed);

    // Initialize the vector that will contain the generators.
    let mut generators = Vec::new();

    // Generating the generators.
    for _ in 0..number {
        generators.push(P::G1::rand(&mut rng));
    }

    generators
}

pub fn pedersen_generator_powers<W: Window, P: Pairing>(
    personalization: u64,
) -> PedersenParameters<P::G1> {
    let generators = pedersen_generators::<P>(W::NUM_WINDOWS + 1, personalization);
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
