use algebra::mnt4_753::Parameters;
use algebra::mnt6_753::Fr as MNT6Fr;
use algebra_core::curves::models::mnt4::MNT4Parameters;
use r1cs_std::mnt4_753::FqGadget;

use crate::gadgets::pedersen::{PedersenCommitmentGadget as PCG, PedersenHashGadget as PHG};

pub type PedersenCommitmentGadget =
    PCG<<Parameters as MNT4Parameters>::G1Parameters, MNT6Fr, FqGadget>;

pub type PedersenHashGadget = PHG<<Parameters as MNT4Parameters>::G1Parameters, MNT6Fr, FqGadget>;
