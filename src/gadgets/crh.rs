use algebra::bls12_377::{Fq, G1Affine, G1Projective};
use algebra::AffineCurve;
use algebra_core::ProjectiveCurve;
use crypto_primitives::crh::pedersen::{
    constraints::PedersenCRHGadget, constraints::PedersenCRHGadgetParameters, PedersenCRH,
    PedersenParameters, PedersenWindow,
};
use r1cs_std::bls12_377::G1Gadget;

use crate::constants::{
    G1_GENERATOR1, G1_GENERATOR2, G1_GENERATOR3, G1_GENERATOR4, G1_GENERATOR5, G1_GENERATOR6,
    G1_GENERATOR7, G1_GENERATOR8, VALIDATOR_SLOTS,
};

pub type CRH = PedersenCRH<G1Projective, CRHWindow>;

pub type CRHGadget = PedersenCRHGadget<G1Projective, Fq, G1Gadget>;

pub type CRHGadgetParameters = PedersenCRHGadgetParameters<G1Projective, CRHWindow, Fq, G1Gadget>;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CRHWindow;

// The input is composed of:
// - Round number: 1 byte
// - Block number: 4 bytes
// - Header hash: 32 bytes
// - Validator public keys: validator slots * public key size bytes
impl PedersenWindow for CRHWindow {
    const WINDOW_SIZE: usize = 1 + 4 + 32 + VALIDATOR_SLOTS * 96;
    const NUM_WINDOWS: usize = 8;
}

pub fn setup_crh<W: PedersenWindow>() -> PedersenParameters<G1Projective> {
    let mut base_generators: Vec<G1Affine> = vec![];
    base_generators.push(G1_GENERATOR1.clone());
    base_generators.push(G1_GENERATOR2.clone());
    base_generators.push(G1_GENERATOR3.clone());
    base_generators.push(G1_GENERATOR4.clone());
    base_generators.push(G1_GENERATOR5.clone());
    base_generators.push(G1_GENERATOR6.clone());
    base_generators.push(G1_GENERATOR7.clone());
    base_generators.push(G1_GENERATOR8.clone());
    assert!(W::NUM_WINDOWS <= base_generators.len());

    let mut generators = Vec::new();
    for i in 0..W::NUM_WINDOWS {
        let mut generators_for_segment = Vec::new();
        let mut base = base_generators[i].into_projective();
        for _ in 0..W::WINDOW_SIZE {
            generators_for_segment.push(base);
            for _ in 0..4 {
                ProjectiveCurve::double_in_place(&mut base);
            }
        }
        generators.push(generators_for_segment);
    }

    PedersenParameters { generators }
}
