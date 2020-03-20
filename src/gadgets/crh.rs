use algebra::bls12_377::{Fq, G1Projective};
use crypto_primitives::crh::pedersen::constraints::{
    PedersenCRHGadget, PedersenCRHGadgetParameters,
};
use r1cs_std::bls12_377::G1Gadget;

use crate::primitives::CRHWindow;

pub type CRHGadget = PedersenCRHGadget<G1Projective, Fq, G1Gadget>;

pub type CRHGadgetParameters = PedersenCRHGadgetParameters<G1Projective, CRHWindow, Fq, G1Gadget>;
