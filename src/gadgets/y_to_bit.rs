use std::ops::Neg;

use algebra::{bls12_377::Fq, sw6::Fr as SW6Fr, One, PrimeField};
use r1cs_core::SynthesisError;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::bls12_377::{FqGadget, G1Gadget, G2Gadget};
use r1cs_std::prelude::{AllocGadget, FieldGadget};
use r1cs_std::{Assignment, ToBitsGadget};

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library.
pub struct YToBitGadget;

impl YToBitGadget {
    pub fn y_to_bit_g1<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        pk: &G1Gadget,
    ) -> Result<Boolean, SynthesisError> {
        let half_plus_one_neg = (Fq::from_repr(Fq::modulus_minus_one_div_two()) + &Fq::one()).neg();
        let y_bit = Boolean::alloc(cs.ns(|| "alloc y bit"), || {
            if pk.y.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(pk.y.get_value().get()?.into_repr() > half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
        let y_adjusted = FqGadget::alloc(cs.ns(|| "alloc y"), || {
            if pk.y.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                let y_value = pk.y.get_value().get()?;
                if y_value.into_repr() > half {
                    Ok(y_value - &(Fq::from_repr(half) + &Fq::one()))
                } else {
                    Ok(y_value)
                }
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
        let y_bit_lc = y_bit.lc(CS::one(), half_plus_one_neg);
        cs.enforce(
            || "check y bit",
            |lc| lc + (Fq::one(), CS::one()),
            |lc| pk.y.get_variable() + y_bit_lc + lc,
            |lc| y_adjusted.get_variable() + lc,
        );
        let y_adjusted_bits = &y_adjusted.to_bits(cs.ns(|| "y adjusted to bits"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, Fq, _>(
            cs.ns(|| "enforce smaller than modulus minus one div two"),
            y_adjusted_bits,
            Fq::modulus_minus_one_div_two(),
        )?;
        Ok(y_bit)
    }

    pub fn y_to_bit_g2<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        pk: &G2Gadget,
    ) -> Result<Boolean, SynthesisError> {
        let half_plus_one_neg = (Fq::from_repr(Fq::modulus_minus_one_div_two()) + &Fq::one()).neg();
        let y_c1_bit = Boolean::alloc(cs.ns(|| "alloc y c1 bit"), || {
            if pk.y.c1.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(pk.y.c1.get_value().get()?.into_repr() > half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
        let y_c0_bit = Boolean::alloc(cs.ns(|| "alloc y c0 bit"), || {
            if pk.y.c0.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(pk.y.c0.get_value().get()?.into_repr() > half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
        let y_eq_bit = Boolean::alloc(cs.ns(|| "alloc y eq bit"), || {
            if pk.y.c0.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(pk.y.c0.get_value().get()?.into_repr() == half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
        let y_bit = Boolean::alloc(cs.ns(|| "alloc y bit"), || {
            if pk.y.c1.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                if pk.y.c1.get_value().get()?.into_repr() > half {
                    Ok(true)
                } else if pk.y.c1.get_value().get()?.into_repr() == half
                    && pk.y.c0.get_value().get()?.into_repr() > half
                {
                    Ok(true)
                } else {
                    Ok(false)
                }
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
        let y_c1_adjusted = FqGadget::alloc(cs.ns(|| "alloc y c1"), || {
            if pk.y.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                let y_value = pk.y.c1.get_value().get()?;
                if y_value.into_repr() > half {
                    Ok(y_value - &(Fq::from_repr(half) + &Fq::one()))
                } else {
                    Ok(y_value)
                }
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
        let y_c0_adjusted = FqGadget::alloc(cs.ns(|| "alloc y c0"), || {
            if pk.y.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                let y_value = pk.y.c0.get_value().get()?;
                if y_value.into_repr() > half {
                    Ok(y_value - &(Fq::from_repr(half) + &Fq::one()))
                } else {
                    Ok(y_value)
                }
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
        let y_c1_bit_lc = y_c1_bit.lc(CS::one(), half_plus_one_neg);
        cs.enforce(
            || "check y bit c1",
            |lc| lc + (Fq::one(), CS::one()),
            |lc| pk.y.c1.get_variable() + y_c1_bit_lc + lc,
            |lc| y_c1_adjusted.get_variable() + lc,
        );
        let y_c1_adjusted_bits = &y_c1_adjusted.to_bits(cs.ns(|| "y c1 adjusted to bits"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, Fq, _>(
            cs.ns(|| "enforce y c1 smaller than modulus minus one div two"),
            y_c1_adjusted_bits,
            Fq::modulus_minus_one_div_two(),
        )?;
        let y_c0_bit_lc = y_c0_bit.lc(CS::one(), half_plus_one_neg);
        cs.enforce(
            || "check y bit c0",
            |lc| lc + (Fq::one(), CS::one()),
            |lc| pk.y.c0.get_variable() + y_c0_bit_lc + lc,
            |lc| y_c0_adjusted.get_variable() + lc,
        );
        let y_c0_adjusted_bits = &y_c0_adjusted.to_bits(cs.ns(|| "y c0 adjusted to bits"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, Fq, _>(
            cs.ns(|| "enforce y c0 smaller than modulus minus one div two"),
            y_c0_adjusted_bits,
            Fq::modulus_minus_one_div_two(),
        )?;

        // (1-a)*(b*c) == o - a
        // a is c1
        // b is y_eq
        // c is c0

        let bc = Boolean::and(cs.ns(|| "and bc"), &y_eq_bit, &y_c0_bit)?;

        cs.enforce(
            || "enforce y bit derived correctly",
            |lc| lc + (Fq::one(), CS::one()) + y_c1_bit.lc(CS::one(), Fq::one().neg()),
            |_| bc.lc(CS::one(), Fq::one()),
            |lc| lc + y_bit.lc(CS::one(), Fq::one()) + y_c1_bit.lc(CS::one(), Fq::one().neg()),
        );

        Ok(y_bit)
    }
}
