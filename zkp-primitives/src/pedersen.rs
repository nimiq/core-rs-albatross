use std::sync::OnceLock;

use ark_crypto_primitives::crh::{
    pedersen::{Window, CRH},
    CRHScheme,
};
use ark_ec::{AffineRepr, CurveGroup};
use ark_mnt6_753::G1Projective;
use nimiq_pedersen_generators::{default, DefaultWindow, PedersenParameters};

pub fn pedersen_parameters() -> &'static PedersenParameters<G1Projective> {
    static CACHE: OnceLock<PedersenParameters<G1Projective>> = OnceLock::new();
    CACHE.get_or_init(default)
}

/// Calculates the Pedersen hash. Given a vector of bits b_i we divide the vector into chunks
/// of 752 bits and convert them into scalars like so:
/// s = b_0 * 2^0 + b_1 * 2^1 + ... + b_750 * 2^750 + b_751 * 2^751
/// We then calculate the commitment like so:
/// H = G_0 + s_1 * G_1 + ... + s_n * G_n
/// where G_0 is a generator that is used as a blinding factor.
pub fn default_pedersen_hash(input: &[u8]) -> G1Projective {
    pedersen_hash::<_, DefaultWindow>(input, pedersen_parameters())
}

pub fn pedersen_hash<C: CurveGroup, W: Window>(
    input: &[u8],
    parameters: &PedersenParameters<C>,
) -> C {
    let mut hash = CRH::<C, W>::evaluate(&parameters.parameters, input)
        .unwrap()
        .into_group();
    hash += &parameters.blinding_factor;
    hash
}
