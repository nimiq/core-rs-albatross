use algebra::bls12_377::{G1Affine, G1Projective};
use algebra::AffineCurve;
use algebra_core::ProjectiveCurve;
use crypto_primitives::crh::pedersen::{PedersenCRH, PedersenParameters, PedersenWindow};

use crate::constants::{
    G1_GENERATOR1, G1_GENERATOR2, G1_GENERATOR3, G1_GENERATOR4, G1_GENERATOR5, G1_GENERATOR6,
    G1_GENERATOR7, G1_GENERATOR8, VALIDATOR_SLOTS,
};

/// A type representing the Pedersen hash. It can be used to call the associated methods of the
/// PedersenCRH struct.
pub type CRH = PedersenCRH<G1Projective, CRHWindow>;

/// A struct that is supposed to contain the number of windows and the window size parameters for our
/// instance of the Pedersen hash function.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CRHWindow;

/// Our input to the Pedersen hash function will be composed of:
/// - Round number: 1 byte
/// - Block number: 4 bytes
/// - Header hash: 32 bytes
/// - Validator public keys: validator slots * public key size bytes
/// We need to do this precalculation because Pedersen is a fixed-length hash function, we need to know the
/// length of the input before we create the parameters.
/// Note: We always set the nunmber of windows to be 8 and adjust the window size. That's because increasing the
/// number of windows would require creating more generator points and that's a slow process. Furthermore, we
/// know that our input will always be a multiple of 8 (since 1 byte = 8 bits) so it's just convenient.
impl PedersenWindow for CRHWindow {
    const WINDOW_SIZE: usize = 1 + 4 + 32 + VALIDATOR_SLOTS * 96;
    const NUM_WINDOWS: usize = 8;
}

/// Function for generating the Pedersen hash parameters for our specific instance. The parameters in this case
/// refer to the vector of generator points, one for each bit of the input.
pub fn setup_crh() -> PedersenParameters<G1Projective> {
    let mut base_generators: Vec<G1Affine> = vec![];
    base_generators.push(G1_GENERATOR1.clone());
    base_generators.push(G1_GENERATOR2.clone());
    base_generators.push(G1_GENERATOR3.clone());
    base_generators.push(G1_GENERATOR4.clone());
    base_generators.push(G1_GENERATOR5.clone());
    base_generators.push(G1_GENERATOR6.clone());
    base_generators.push(G1_GENERATOR7.clone());
    base_generators.push(G1_GENERATOR8.clone());
    assert!(CRHWindow::NUM_WINDOWS <= base_generators.len());

    let mut generators = Vec::new();
    for i in 0..CRHWindow::NUM_WINDOWS {
        let mut generators_for_segment = Vec::new();
        let mut base = base_generators[i].into_projective();
        for _ in 0..CRHWindow::WINDOW_SIZE {
            generators_for_segment.push(base);
            for _ in 0..4 {
                ProjectiveCurve::double_in_place(&mut base);
            }
        }
        generators.push(generators_for_segment);
    }

    PedersenParameters { generators }
}
