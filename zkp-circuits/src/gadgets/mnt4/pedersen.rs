use ark_mnt4_753::{constraints::G1Var, G1Projective};

use crate::gadgets::pedersen::PedersenParametersVar;

pub type DefaultPedersenParametersVar = PedersenParametersVar<G1Projective, G1Var>;
