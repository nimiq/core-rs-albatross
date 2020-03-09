use algebra::curves::bls12_377::{G1Affine, G1Projective};
use algebra::fields::bls12_377::Fq;
use algebra::{AffineCurve, Group, PrimeField};
use crypto_primitives::crh::pedersen::{PedersenCRH, PedersenParameters, PedersenWindow};

use crate::setup::{
    read_bigint384_const, G1_X_1, G1_X_2, G1_X_3, G1_X_4, G1_X_5, G1_X_6, G1_X_7, G1_X_8, G1_Y_1,
    G1_Y_2, G1_Y_3, G1_Y_4, G1_Y_5, G1_Y_6, G1_Y_7, G1_Y_8,
};

pub type CRH<T> = PedersenCRH<G1Projective, T>;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CRHWindowBlock;

// Parameters are 1 + 4 + 32 + 512 * 96 = 49189 bytes
// Our fixed-length input is xxx bits.
impl PedersenWindow for CRHWindowBlock {
    const WINDOW_SIZE: usize = 49189;
    const NUM_WINDOWS: usize = 8;
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CRHWindowState;

// Parameters are 4 + 512 * 96 = 49156 bytes
// Our fixed-length input is xxx bits.
impl PedersenWindow for CRHWindowState {
    const WINDOW_SIZE: usize = 49156;
    const NUM_WINDOWS: usize = 8;
}

pub fn setup_crh<W: PedersenWindow>() -> PedersenParameters<G1Projective> {
    let mut base_generators = vec![];
    let x = Fq::from_repr(read_bigint384_const(G1_X_1));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_1));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_2));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_2));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_3));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_3));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_4));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_4));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_5));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_5));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_6));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_6));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_7));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_7));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_8));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_8));
    base_generators.push(G1Affine::new(x, y, false));
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
