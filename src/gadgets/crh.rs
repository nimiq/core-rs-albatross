use algebra::bls12_377::G1Projective;
use algebra::sw6::Fr as SW6Fr;
use crypto_primitives::{crh::pedersen::constraints::PedersenCRHGadget, FixedLengthCRHGadget};
use r1cs_std::bls12_377::G1Gadget;

use crate::primitives::CRH;

pub type CRHGadget = PedersenCRHGadget<G1Projective, SW6Fr, G1Gadget>;

pub type CRHGadgetParameters = <CRHGadget as FixedLengthCRHGadget<CRH, SW6Fr>>::ParametersGadget;
