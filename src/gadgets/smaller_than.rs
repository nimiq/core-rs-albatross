use std::marker::PhantomData;

use algebra::{Field, PrimeField};
use r1cs_core::{ConstraintSystem, SynthesisError};
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::prelude::FieldGadget;
use r1cs_std::ToBitsGadget;

pub struct SmallerThanGadget<ConstraintF: Field + PrimeField> {
    constraint_field_type: PhantomData<ConstraintF>,
}

impl<ConstraintF: Field + PrimeField> SmallerThanGadget<ConstraintF> {
    // the function assumes a and b are known to be <= (p-1)/2
    pub fn is_smaller_than<CS: ConstraintSystem<ConstraintF>>(
        mut cs: CS,
        a: &FpGadget<ConstraintF>,
        b: &FpGadget<ConstraintF>,
    ) -> Result<Boolean, SynthesisError> {
        let two = ConstraintF::one() + &ConstraintF::one();
        let d0 = a.sub(cs.ns(|| "a - b"), b)?;
        let d = d0.mul_by_constant(cs.ns(|| "mul 2"), &two)?;
        let d_bits = d.to_bits_strict(cs.ns(|| "d to bits"))?;
        let d_bits_len = d_bits.len();
        Ok(d_bits[d_bits_len - 1])
    }

    // the function assumes a and b are known to be <= (p-1)/2
    pub fn enforce_smaller_than<CS: ConstraintSystem<ConstraintF>>(
        mut cs: CS,
        a: &FpGadget<ConstraintF>,
        b: &FpGadget<ConstraintF>,
    ) -> Result<(), SynthesisError> {
        let is_smaller_than = Self::is_smaller_than(cs.ns(|| "is smaller than"), a, b)?;
        cs.enforce(
            || "enforce smaller than",
            |_| is_smaller_than.lc(CS::one(), ConstraintF::one()),
            |lc| lc + (ConstraintF::one(), CS::one()),
            |lc| lc + (ConstraintF::one(), CS::one()),
        );

        Ok(())
    }

    pub fn enforce_smaller_than_strict<CS: ConstraintSystem<ConstraintF>>(
        mut cs: CS,
        a: &FpGadget<ConstraintF>,
        b: &FpGadget<ConstraintF>,
    ) -> Result<(), SynthesisError> {
        let a_bits = a.to_bits(cs.ns(|| "a to bits"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, ConstraintF, _>(
            cs.ns(|| "enforce a smaller than modulus minus one div two"),
            &a_bits,
            ConstraintF::modulus_minus_one_div_two(),
        )?;
        let b_bits = b.to_bits(cs.ns(|| "b to bits"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, ConstraintF, _>(
            cs.ns(|| "enforce b smaller than modulus minus one div two"),
            &b_bits,
            ConstraintF::modulus_minus_one_div_two(),
        )?;

        let is_smaller_than = Self::is_smaller_than(cs.ns(|| "is smaller than"), a, b)?;
        cs.enforce(
            || "enforce smaller than",
            |_| is_smaller_than.lc(CS::one(), ConstraintF::one()),
            |lc| lc + (ConstraintF::one(), CS::one()),
            |lc| lc + (ConstraintF::one(), CS::one()),
        );

        Ok(())
    }
}
