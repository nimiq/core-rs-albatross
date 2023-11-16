use ark_mnt4_753::{constraints::G1Var, G1Projective};
use nimiq_pedersen_generators::DefaultWindow;

use crate::gadgets::pedersen::{PedersenHashGadget, PedersenParametersVar};

pub type DefaultPedersenParametersVar = PedersenParametersVar<G1Projective, G1Var>;
pub type DefaultPedersenHashGadget = PedersenHashGadget<G1Projective, G1Var, DefaultWindow>;
