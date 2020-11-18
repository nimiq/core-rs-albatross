use algebra::mnt4_753::Fr as MNT4Fr;
use algebra::mnt6_753::Parameters;
use algebra_core::curves::models::mnt6::MNT6Parameters;
use r1cs_core::SynthesisError;
use r1cs_std::mnt6_753::{G1Gadget, G2Gadget};
use r1cs_std::prelude::Boolean;

use crate::gadgets::y_to_bit::YToBitGadget as Y2BG;

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library. (https://github.com/celo-org/bls-zexe)
pub type YToBitGadget = Y2BG<<Parameters as MNT6Parameters>::G1Parameters, MNT4Fr>;

impl YToBitGadget {
    /// Outputs a boolean representing the relation:
    /// y > half
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g1<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(mut cs: CS, point: &G1Gadget) -> Result<Boolean, SynthesisError> {
        let y_bit = Self::is_greater_half(&mut cs.ns(|| "calculate y bit"), &point.y)?;

        Ok(y_bit)
    }

    /// Outputs a boolean representing the relation:
    /// (y_c2 > half) || (y_c2 == 0 && y_c1 > half) || (y_c2 == 0 && y_c1 == 0 && y_c0 > half)
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g2<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(mut cs: CS, point: &G2Gadget) -> Result<Boolean, SynthesisError> {
        // Calculate the required inputs to the formula.
        let y_c2_bit = Self::is_greater_half(&mut cs.ns(|| "calculate y_c2_bit"), &point.y.c2)?;

        let y_c1_bit = Self::is_greater_half(&mut cs.ns(|| "calculate y_c1_bit"), &point.y.c1)?;

        let y_c0_bit = Self::is_greater_half(&mut cs.ns(|| "calculate y_c0 bit"), &point.y.c0)?;

        let y_c2_eq_bit = Self::is_equal_zero(&mut cs.ns(|| "calculate y_c2_eq_bit"), &point.y.c2)?;

        let y_c1_eq_bit = Self::is_equal_zero(&mut cs.ns(|| "calculate y_c1_eq_bit"), &point.y.c1)?;

        // Calculate the following formula:
        // (y_c2 > half) || (y_c2 == 0 && y_c1 > half) || (y_c2 == 0 && y_c1 == 0 && y_c0 > half)
        let cond0 = y_c2_bit;

        let cond1 = Boolean::and(cs.ns(|| "y_c2_eq_bit && y_c1_bit"), &y_c2_eq_bit, &y_c1_bit)?;

        let cond2 = Boolean::kary_and(
            cs.ns(|| "y_c2_eq_bit && y_c1_eq_bit && y_c0_bit"),
            vec![y_c2_eq_bit, y_c1_eq_bit, y_c0_bit].as_ref(),
        )?;

        let y_bit = Boolean::kary_or(cs.ns(|| "cond0 || cond1 || cond2"), vec![cond0, cond1, cond2].as_ref())?;

        Ok(y_bit)
    }
}
