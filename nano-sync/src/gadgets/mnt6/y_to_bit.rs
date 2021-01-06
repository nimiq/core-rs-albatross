use ark_mnt4_753::Parameters as MNT4Parameters;
use ark_mnt6_753::Fr as MNT6Fr;
use ark_r1cs_std::boolean::Boolean;
use ark_r1cs_std::groups::mnt4::{G1Var, G2Var};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::y_to_bit::YToBitGadget as Y2BG;

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library. (https://github.com/celo-org/bls-zexe)
pub type YToBitGadget = Y2BG<MNT6Fr>;

impl YToBitGadget {
    /// Outputs a boolean representing the relation:
    /// y > half
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g1(
        cs: ConstraintSystemRef<MNT6Fr>,
        point: &G1Var<MNT4Parameters>,
    ) -> Result<Boolean<MNT6Fr>, SynthesisError> {
        let y_bit = Self::is_greater_half(cs.clone(), &point.y)?;

        Ok(y_bit)
    }

    /// Outputs a boolean representing the relation:
    /// (y_c1 > half) || (y_c1 == 0 && y_c0 > half)
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g2(
        cs: ConstraintSystemRef<MNT6Fr>,
        point: &G2Var<MNT4Parameters>,
    ) -> Result<Boolean<MNT6Fr>, SynthesisError> {
        // Calculate the required inputs to the formula.
        let y_c1_bit = Self::is_greater_half(cs.clone(), &point.y.c1)?;

        let y_c0_bit = Self::is_greater_half(cs.clone(), &point.y.c0)?;

        let y_c1_eq_bit = Self::is_equal_zero(cs.clone(), &point.y.c1)?;

        // Calculate the following formula:
        // (y_c1 > half) || (y_c1 == 0 && y_c0 > half)
        let cond0 = y_c1_bit;

        let cond1 = Boolean::and(&y_c1_eq_bit, &y_c0_bit)?;

        let y_bit = Boolean::or(&cond0, &cond1)?;

        Ok(y_bit)
    }
}
