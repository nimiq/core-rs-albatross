use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::Parameters;
use algebra_core::curves::models::mnt6::MNT6Parameters;
use r1cs_std::mnt6_753::FqGadget;

use crate::gadgets::pedersen::PedersenHashGadget as PHG;

pub type PedersenHashGadget = PHG<<Parameters as MNT6Parameters>::G1Parameters, MNT4Fr, FqGadget>;
