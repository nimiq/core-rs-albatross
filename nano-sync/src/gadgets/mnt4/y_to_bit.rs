use ark_ec::mnt6::MNT6Parameters;
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{Fq3Var, FqVar};
use ark_mnt6_753::Parameters;
use ark_r1cs_std::groups::curves::short_weierstrass::AffineVar;
use ark_r1cs_std::prelude::Boolean;
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::y_to_bit::YToBitGadget as Y2BG;

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library. (https://github.com/celo-org/bls-zexe)
pub type YToBitGadget = Y2BG<MNT4Fr>;

impl YToBitGadget {
    /// Outputs a boolean representing the relation:
    /// y > half
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g1(
        cs: ConstraintSystemRef<MNT4Fr>,
        point: &AffineVar<<Parameters as MNT6Parameters>::G1Parameters, FqVar>,
    ) -> Result<Boolean<MNT4Fr>, SynthesisError> {
        let y_bit = Self::is_greater_half(cs.clone(), &point.y)?;

        Ok(y_bit)
    }

    /// Outputs a boolean representing the relation:
    /// (y_c2 > half) || (y_c2 == 0 && y_c1 > half) || (y_c2 == 0 && y_c1 == 0 && y_c0 > half)
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g2(
        cs: ConstraintSystemRef<MNT4Fr>,
        point: &AffineVar<<Parameters as MNT6Parameters>::G2Parameters, Fq3Var>,
    ) -> Result<Boolean<MNT4Fr>, SynthesisError> {
        // Calculate the required inputs to the formula.
        let y_c2_bit = Self::is_greater_half(cs.clone(), &point.y.c2)?;

        let y_c1_bit = Self::is_greater_half(cs.clone(), &point.y.c1)?;

        let y_c0_bit = Self::is_greater_half(cs.clone(), &point.y.c0)?;

        let y_c2_eq_bit = Self::is_equal_zero(cs.clone(), &point.y.c2)?;

        let y_c1_eq_bit = Self::is_equal_zero(cs.clone(), &point.y.c1)?;

        // Calculate the following formula:
        // (y_c2 > half) || (y_c2 == 0 && y_c1 > half) || (y_c2 == 0 && y_c1 == 0 && y_c0 > half)
        let cond0 = y_c2_bit;

        let cond1 = Boolean::and(&y_c2_eq_bit, &y_c1_bit)?;

        let cond2 = Boolean::kary_and(vec![y_c2_eq_bit, y_c1_eq_bit, y_c0_bit].as_ref())?;

        let y_bit = Boolean::kary_or(vec![cond0, cond1, cond2].as_ref())?;

        Ok(y_bit)
    }
}
