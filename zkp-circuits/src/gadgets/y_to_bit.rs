use ark_ff::PrimeField;
use ark_r1cs_std::{
    boolean::Boolean,
    fields::fp::FpVar,
    prelude::{AllocVar, EqGadget, FieldVar},
    R1CSVar, ToBitsGadget,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library. (https://github.com/celo-org/bls-zexe)
pub trait YToBitGadget<BaseField: PrimeField> {
    fn y_to_bit(
        &self,
        cs: ConstraintSystemRef<BaseField>,
    ) -> Result<Boolean<BaseField>, SynthesisError>;

    /// Outputs a boolean representing the relation:
    /// y > half
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    fn is_greater_half(
        cs: ConstraintSystemRef<BaseField>,
        y: &FpVar<BaseField>,
    ) -> Result<Boolean<BaseField>, SynthesisError> {
        // Calculates half.
        let half_value = BaseField::from_bigint(BaseField::MODULUS_MINUS_ONE_DIV_TWO).unwrap();

        // Allocates -half as a constant.
        let half_neg = FpVar::new_constant(cs.clone(), half_value.neg())?;

        // Calculates and allocates the bit representing the "sign" of the y-coordinate for this
        // corresponding field.
        let y_bit = Boolean::new_witness(cs.clone(), || Ok(y.value()? > half_value))?;

        // Converts y_bit to a field element (so you can do arithmetic with it).
        let y_bit_fp = FpVar::from(y_bit.clone());

        // Calculates and allocates the value y_adjusted.
        // This value is necessary so that later we can enforce the correctness of y_bit.
        let y_adjusted = FpVar::new_witness(cs, || {
            let value = y.value()?;

            let adjusted = if value > half_value {
                value - half_value
            } else {
                value
            };

            Ok(adjusted)
        })?;

        // Enforces the following relation:
        // y_adjusted <= half
        let y_adjusted_bits = &y_adjusted.to_bits_le()?;

        Boolean::enforce_smaller_or_equal_than_le(
            y_adjusted_bits,
            BaseField::MODULUS_MINUS_ONE_DIV_TWO,
        )?;

        // Enforces the following relation:
        // y + y_bit * (-half) = y_adjusted
        let lhs = y + y_bit_fp * half_neg;

        lhs.enforce_equal(&y_adjusted)?;

        Ok(y_bit)
    }

    /// Outputs a boolean representing the relation:
    /// y = zero
    /// where zero means the identity element of the underlying field.
    fn is_equal_zero(
        cs: ConstraintSystemRef<BaseField>,
        y: &FpVar<BaseField>,
    ) -> Result<Boolean<BaseField>, SynthesisError> {
        // Calculates and allocates the bit representing if y == 0.
        let y_eq_bit = Boolean::new_witness(cs.clone(), || Ok(y.value()? == BaseField::zero()))?;

        // Converts y_eq_bit to a field element (so you can do arithmetic with it).
        let y_eq_bit_fp = FpVar::from(y_eq_bit.clone());

        // Calculates and allocates the inverse of y.
        // This value is necessary so that later we can enforce the correctness of y_eq_bit.
        let y_inv = FpVar::new_witness(cs, || {
            Ok(y.value()?.inverse().unwrap_or_else(BaseField::zero))
        })?;

        // Enforces the following relation:
        // y * y_inv == 1 - y_eq_bit
        // This guarantees that y and y_eq_bit cannot both be zero.
        let lhs = y * y_inv;

        let rhs = FpVar::one() - y_eq_bit_fp.clone();

        lhs.enforce_equal(&rhs)?;

        // Enforces the following relation:
        // y * y_eq_bit == 0
        // This guarantees that y and y_eq_bit cannot both be different from zero.
        let lhs = y * y_eq_bit_fp;

        lhs.enforce_equal(&FpVar::zero())?;

        Ok(y_eq_bit)
    }
}
