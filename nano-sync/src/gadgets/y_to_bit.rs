use std::marker::PhantomData;

use algebra_core::{PrimeField, SWModelParameters};
use r1cs_core::SynthesisError;
use r1cs_std::boolean::Boolean;
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::fields::FieldGadget;
use r1cs_std::prelude::AllocGadget;
use r1cs_std::{Assignment, ToBitsGadget};

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library. (https://github.com/celo-org/bls-zexe)
pub struct YToBitGadget<P: SWModelParameters, F: PrimeField> {
    _params: PhantomData<P>,
    _cfield: PhantomData<F>,
}

impl<P: SWModelParameters, F: PrimeField> YToBitGadget<P, F> {
    /// Outputs a boolean representing the relation:
    /// y > half
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn is_greater_half<CS: r1cs_core::ConstraintSystem<F>>(mut cs: CS, y: &FpGadget<F>) -> Result<Boolean, SynthesisError> {
        // Calculates half.
        let half = F::from_repr(F::modulus_minus_one_div_two());

        // Calculates and allocates the bit representing the "sign" of the y-coordinate for this
        // corresponding field.
        let y_bit = Boolean::alloc(cs.ns(|| "alloc y bit"), || Ok(y.get_value().get()? > half))?;

        // Calculates and allocates the value y_adjusted.
        // This value is necessary so that later we can enforce the correctness of y_bit.
        let y_adjusted = FpGadget::<F>::alloc(cs.ns(|| "alloc y adjusted"), || {
            let value = y.get_value().get()?;

            let adjusted = if value > half { value - &half } else { value };

            Ok(adjusted)
        })?;

        // Enforces the following relation:
        // y_adjusted <= half
        let y_adjusted_bits = &y_adjusted.to_bits(cs.ns(|| "y adjusted to bits"))?;

        Boolean::enforce_smaller_or_equal_than::<_, _, F, _>(
            cs.ns(|| "enforce y adjusted smaller than half"),
            y_adjusted_bits,
            F::modulus_minus_one_div_two(),
        )?;

        // Enforces the following relation:
        // y + y_bit * (-half) = y_adjusted
        let y_bit_lc = y_bit.lc(CS::one(), half.neg());

        cs.enforce(
            || "check y bit",
            |lc| lc + (F::one(), CS::one()),
            |lc| y.get_variable() + y_bit_lc + lc,
            |lc| y_adjusted.get_variable() + lc,
        );

        Ok(y_bit)
    }

    /// Outputs a boolean representing the relation:
    /// y = zero
    /// where zero means the identity element of the underlying field.
    pub fn is_equal_zero<CS: r1cs_core::ConstraintSystem<F>>(mut cs: CS, y: &FpGadget<F>) -> Result<Boolean, SynthesisError> {
        // Calculates and allocates the bit representing if y == 0.
        let y_eq_bit = Boolean::alloc(cs.ns(|| "alloc y eq bit"), || Ok(y.get_value().get()? == F::zero()))?;

        // Calculates and allocates the inverse of y.
        // This value is necessary so that later we can enforce the correctness of y_eq_bit.
        let inv = FpGadget::<F>::alloc(cs.ns(|| "alloc y inv"), || Ok(y.get_value().get()?.inverse().unwrap_or_else(F::zero)))?;

        // Enforces the following relation:
        // y * y_inv == 1 - y_eq_bit
        // This guarantees that y and y_eq_bit cannot both be zero.
        cs.enforce(
            || "enforce y_eq_bit",
            |lc| y.get_variable() + lc,
            |lc| inv.get_variable() + lc,
            |lc| lc + (F::one(), CS::one()) + y_eq_bit.lc(CS::one(), F::one().neg()),
        );

        // Enforces the following relation:
        // y * y_eq_bit == 0
        // This guarantees that y and y_eq_bit cannot both be different from zero.
        cs.enforce(
            || "enforce y_eq_bit 2",
            |lc| y.get_variable() + lc,
            |_| y_eq_bit.lc(CS::one(), F::one()),
            |lc| lc,
        );

        Ok(y_eq_bit)
    }
}
