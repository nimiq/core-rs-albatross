use algebra::mnt4_753::Parameters;
use algebra::mnt6_753::Fr as MNT6Fr;
use algebra_core::curves::models::mnt4::MNT4Parameters;
use r1cs_core::SynthesisError;
use r1cs_std::mnt4_753::{G1Gadget, G2Gadget};
use r1cs_std::prelude::Boolean;

use crate::gadgets::y_to_bit::YToBitGadget as Y2BG;

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library. (https://github.com/celo-org/bls-zexe)
pub type YToBitGadget = Y2BG<<Parameters as MNT4Parameters>::G1Parameters, MNT6Fr>;

impl YToBitGadget {
    /// Outputs a boolean representing the relation:
    /// y > half
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g1<CS: r1cs_core::ConstraintSystem<MNT6Fr>>(
        mut cs: CS,
        point: &G1Gadget,
    ) -> Result<Boolean, SynthesisError> {
        let y_bit = Self::is_greater_half(&mut cs.ns(|| "calculate y bit"), &point.y)?;

        Ok(y_bit)
    }

    /// Outputs a boolean representing the relation:
    /// (y_c1 > half) || (y_c1 == 0 && y_c0 > half)
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g2<CS: r1cs_core::ConstraintSystem<MNT6Fr>>(
        mut cs: CS,
        point: &G2Gadget,
    ) -> Result<Boolean, SynthesisError> {
        // Calculate the required inputs to the formula.
        let y_c1_bit = Self::is_greater_half(&mut cs.ns(|| "calculate y_c1_bit"), &point.y.c1)?;

        let y_c0_bit = Self::is_greater_half(&mut cs.ns(|| "calculate y_c0 bit"), &point.y.c0)?;

        let y_c1_eq_bit = Self::is_equal_zero(&mut cs.ns(|| "calculate y_c1_eq_bit"), &point.y.c1)?;

        // Calculate the following formula:
        // (y_c1 > half) || (y_c1 == 0 && y_c0 > half)
        let cond0 = y_c1_bit;

        let cond1 = Boolean::and(cs.ns(|| "y_c1_eq_bit && y_c0_bit"), &y_c1_eq_bit, &y_c0_bit)?;

        let y_bit = Boolean::or(cs.ns(|| "cond0 || cond1"), &cond0, &cond1)?;

        Ok(y_bit)
    }
}
