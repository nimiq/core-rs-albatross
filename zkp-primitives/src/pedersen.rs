use ark_crypto_primitives::crh::pedersen::{bytes_to_bits, Window};
use ark_ec::CurveGroup;
use ark_mnt6_753::G1Projective;
use lazy_static::lazy_static;
use nimiq_pedersen_generators::{default, DefaultWindow, PedersenParameters};

lazy_static! {
    pub static ref PEDERSEN_PARAMETERS: PedersenParameters<G1Projective> = default();
}

/// Calculates the Pedersen hash. Given a vector of bits b_i we divide the vector into chunks
/// of 752 bits and convert them into scalars like so:
/// s = b_0 * 2^0 + b_1 * 2^1 + ... + b_750 * 2^750 + b_751 * 2^751
/// We then calculate the commitment like so:
/// H = G_0 + s_1 * G_1 + ... + s_n * G_n
/// where G_0 is a generator that is used as a blinding factor.
pub fn default_pedersen_hash(input: &[u8]) -> G1Projective {
    pedersen_hash::<_, DefaultWindow>(input, &PEDERSEN_PARAMETERS)
}

pub fn pedersen_hash<C: CurveGroup, W: Window>(
    input: &[u8],
    parameters: &PedersenParameters<C>,
) -> C {
    // The Pedersen Hash implementation of arkworks is not working properly for the mnt6 curve.
    // Thus, we still use our own implementation for now.

    // Check that the input can be stored using the available generators.
    assert!(parameters.parameters.generators.len() >= W::NUM_WINDOWS);
    assert!(W::NUM_WINDOWS * W::WINDOW_SIZE >= input.len() / 8);

    let bits = bytes_to_bits(input);

    // Initialize the sum to the first generator.
    let mut result = parameters.blinding_factor;

    let generator_powers = &parameters.parameters.generators;

    // Start calculating the Pedersen hash.
    for (i, chunk) in bits.chunks(W::WINDOW_SIZE).enumerate() {
        // We multiply each generator by the corresponding scalar formed from 752 bits of input. This
        // is the double-and-add method for EC point multiplication.
        // (https://en.wikipedia.org/wiki/Elliptic_curve_point_multiplication#Double-and-add)
        for (j, bit) in chunk.iter().enumerate() {
            if *bit {
                result += generator_powers[i][j];
            }
        }
    }

    result
}
