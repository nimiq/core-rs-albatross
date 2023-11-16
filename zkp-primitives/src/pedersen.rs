use std::sync::OnceLock;

use ark_crypto_primitives::crh::{
    pedersen::{Window, CRH},
    CRHScheme,
};
use ark_ec::{pairing::Pairing, AffineRepr, CurveGroup};
use ark_mnt4_753::{G1Projective as MNT4_G1Projective, MNT4_753};
use ark_mnt6_753::{G1Projective as MNT6_G1Projective, MNT6_753};
use ark_serialize::CanonicalSerialize;
use nimiq_pedersen_generators::{default_mnt4, default_mnt6, DefaultWindow, PedersenParameters};

pub fn pedersen_parameters_mnt6() -> &'static PedersenParameters<MNT6_G1Projective> {
    static CACHE: OnceLock<PedersenParameters<MNT6_G1Projective>> = OnceLock::new();
    CACHE.get_or_init(default_mnt6)
}

pub fn pedersen_parameters_mnt4() -> &'static PedersenParameters<MNT4_G1Projective> {
    static CACHE: OnceLock<PedersenParameters<MNT4_G1Projective>> = OnceLock::new();
    CACHE.get_or_init(default_mnt4)
}

pub trait DefaultPedersenParameters95: Pairing {
    fn pedersen_parameters() -> &'static PedersenParameters<Self::G1>;
    fn g1_to_bytes(g1: &Self::G1) -> [u8; 95];
}

impl DefaultPedersenParameters95 for MNT4_753 {
    fn pedersen_parameters() -> &'static PedersenParameters<Self::G1> {
        pedersen_parameters_mnt4()
    }

    fn g1_to_bytes(g1: &Self::G1) -> [u8; 95] {
        let mut buffer = [0u8; 95];
        CanonicalSerialize::serialize_compressed(&g1.into_affine(), &mut &mut buffer[..]).unwrap();
        buffer
    }
}

impl DefaultPedersenParameters95 for MNT6_753 {
    fn pedersen_parameters() -> &'static PedersenParameters<Self::G1> {
        pedersen_parameters_mnt6()
    }

    fn g1_to_bytes(g1: &Self::G1) -> [u8; 95] {
        let mut buffer = [0u8; 95];
        CanonicalSerialize::serialize_compressed(&g1.into_affine(), &mut &mut buffer[..]).unwrap();
        buffer
    }
}

/// Calculates the Pedersen hash. Given a vector of bits b_i we divide the vector into chunks
/// of 752 bits and convert them into scalars like so:
/// s = b_0 * 2^0 + b_1 * 2^1 + ... + b_750 * 2^750 + b_751 * 2^751
/// We then calculate the commitment like so:
/// H = G_0 + s_1 * G_1 + ... + s_n * G_n
/// where G_0 is a generator that is used as a blinding factor.
pub fn default_pedersen_hash<P: Pairing + DefaultPedersenParameters95>(input: &[u8]) -> P::G1 {
    pedersen_hash::<_, DefaultWindow>(input, P::pedersen_parameters())
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
