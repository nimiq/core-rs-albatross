use std::ops::Neg;

use algebra::{mnt4_753::Fr as MNT4Fr, mnt6_753::Fq, One, PrimeField};
use r1cs_core::SynthesisError;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::eq::{ConditionalEqGadget, ConditionalOrEqualsGadget, EqGadget};
use r1cs_std::mnt6_753::{FqGadget, G1Gadget, G2Gadget};
use r1cs_std::prelude::{AllocGadget, FieldGadget};
use r1cs_std::select::CondSelectGadget;
use r1cs_std::{Assignment, ToBitsGadget};

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library.
pub struct YToBitGadget;

impl YToBitGadget {
    pub fn y_to_bit_g1<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
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

    pub fn y_to_bit_g2<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        pk: &G2Gadget,
    ) -> Result<Boolean, SynthesisError> {
        let half_plus_one_neg = (Fq::from_repr(Fq::modulus_minus_one_div_two()) + &Fq::one()).neg();
        let y_c2_bit = Boolean::alloc(cs.ns(|| "alloc y c2 bit"), || {
            if pk.y.c2.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(pk.y.c2.get_value().get()?.into_repr() > half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
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
        let y_eq_bit_c2 = Boolean::alloc(cs.ns(|| "alloc y c2 eq bit"), || {
            if pk.y.c2.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(pk.y.c2.get_value().get()?.into_repr() == half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
        let y_eq_bit_c1 = Boolean::alloc(cs.ns(|| "alloc y c1 eq bit"), || {
            if pk.y.c1.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(pk.y.c1.get_value().get()?.into_repr() == half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;
        let y_bit = Boolean::alloc(cs.ns(|| "alloc y bit"), || {
            if pk.y.c1.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                if pk.y.c2.get_value().get()?.into_repr() > half {
                    Ok(true)
                } else if pk.y.c2.get_value().get()?.into_repr() == half
                    && pk.y.c1.get_value().get()?.into_repr() > half
                {
                    Ok(true)
                } else if pk.y.c2.get_value().get()?.into_repr() == half
                    && pk.y.c1.get_value().get()?.into_repr() == half
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
        let y_c2_adjusted = FqGadget::alloc(cs.ns(|| "alloc y c2"), || {
            if pk.y.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                let y_value = pk.y.c2.get_value().get()?;
                if y_value.into_repr() > half {
                    Ok(y_value - &(Fq::from_repr(half) + &Fq::one()))
                } else {
                    Ok(y_value)
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
        let y_c2_bit_lc = y_c2_bit.lc(CS::one(), half_plus_one_neg);
        // c2 + y_c2_bit * (-half + 1) = y_c2_adjusted
        // if c2 < half
        //   c2 + 0 = c2
        // else
        //   c2 - half + 1 = y_c2_adjusted
        cs.enforce(
            || "check y bit c2",
            |lc| lc + (Fq::one(), CS::one()),
            |lc| pk.y.c2.get_variable() + y_c2_bit_lc + lc,
            |lc| y_c2_adjusted.get_variable() + lc,
        );
        let y_c2_adjusted_bits = &y_c2_adjusted.to_bits(cs.ns(|| "y c2 adjusted to bits"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, Fq, _>(
            cs.ns(|| "enforce y c2 smaller than modulus minus one div two"),
            y_c2_adjusted_bits,
            Fq::modulus_minus_one_div_two(),
        )?;
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

        // FIXME: y_eq_bit_{c1,c2} are still unconstrained: https://github.com/celo-org/bls-zexe/issues/147

        // o = if y_eq_bit_c2 { if y_eq_bit_c1 { y_c0_bit } else { y_c1_bit } } else { y_c2_bit }
        let computed_inner_y_bit = Boolean::conditionally_select(
            cs.ns(|| "select based on y_eq_bit_c1"),
            &y_eq_bit_c1,
            &y_c0_bit,
            &y_c1_bit,
        )?;
        let computed_y_bit = Boolean::conditionally_select(
            cs.ns(|| "select based on y_eq_bit_c2"),
            &y_eq_bit_c2,
            &computed_inner_y_bit,
            &y_c2_bit,
        )?;
        y_bit.conditional_enforce_equal(cs.ns(|| "check y bit"), &computed_y_bit)?;

        Ok(y_bit)
    }
}
