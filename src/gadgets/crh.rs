use algebra::curves::bls12_377::{G1Affine, G1Projective};
use algebra::fields::sw6::Fr as SW6Fr;
use algebra::{AffineCurve, Group};
use crypto_primitives::crh::pedersen::constraints::PedersenCRHGadget;
use crypto_primitives::crh::pedersen::{PedersenCRH, PedersenParameters, PedersenWindow};
use crypto_primitives::FixedLengthCRHGadget;
use r1cs_std::groups::curves::short_weierstrass::bls12::bls12_377::G1Gadget;

use crate::constants::{
    G1_GENERATOR1, G1_GENERATOR2, G1_GENERATOR3, G1_GENERATOR4, G1_GENERATOR5, G1_GENERATOR6,
    G1_GENERATOR7, G1_GENERATOR8,
};

pub type CRH<T> = PedersenCRH<G1Projective, T>;

pub type CRHGadget = PedersenCRHGadget<G1Projective, SW6Fr, G1Gadget>;

pub type CRHGadgetParameters =
    <CRHGadget as FixedLengthCRHGadget<CRH<CRHWindow>, SW6Fr>>::ParametersGadget;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CRHWindow;

// TODO: Change to use the validator slots constant.
// Parameters are 1 + 4 + 32 + 512 * 96 = 49189 bytes
// Our fixed-length input is xxx bits.
impl PedersenWindow for CRHWindow {
    const WINDOW_SIZE: usize = 49189;
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
                base.double_in_place();
            }
        }
        generators.push(generators_for_segment);
    }

    PedersenParameters { generators }
}
