use algebra::bls12_377::{Fq, G1Projective};
use crypto_primitives::crh::pedersen::constraints::{
    PedersenCRHGadget, PedersenCRHGadgetParameters,
};
use r1cs_std::bls12_377::G1Gadget;

use crate::primitives::CRHWindow;

/// A type representing a gadget for the Pedersen hash. Necessary to calculate a
/// Pedersen hash in-circuit.
pub type CRHGadget = PedersenCRHGadget<G1Projective, Fq, G1Gadget>;

/// A type representing a gadget for the Pedersen hash parameters. Necessary to calculate a
/// Pedersen hash in-circuit.
pub type CRHGadgetParameters = PedersenCRHGadgetParameters<G1Projective, CRHWindow, Fq, G1Gadget>;
