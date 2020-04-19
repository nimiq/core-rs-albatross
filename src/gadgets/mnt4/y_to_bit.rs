use std::ops::Neg;

use algebra::{mnt4_753::Fr as MNT4Fr, mnt6_753::Fq, One, PrimeField};
use algebra_core::{Field, Zero};
use r1cs_core::SynthesisError;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::fields::fp::FpGadget;
use r1cs_std::mnt6_753::{FqGadget, G1Gadget, G2Gadget};
use r1cs_std::prelude::{AllocGadget, FieldGadget};
use r1cs_std::{Assignment, ToBitsGadget};

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library. (https://github.com/celo-org/bls-zexe)
pub struct YToBitGadget;

impl YToBitGadget {
    /// Outputs a boolean representing the relation:
    /// y > half
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g1<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        point: &G1Gadget,
    ) -> Result<Boolean, SynthesisError> {
        // Calculates -(half + 1), where half = (p-1)/2.
        let half_plus_one_neg = (Fq::from_repr(Fq::modulus_minus_one_div_two()) + &Fq::one()).neg();

        // ----------------   y > half   ----------------
        // Calculates and allocates the bit representing the "sign" of the y-coordinate.
        let y_bit = Boolean::alloc(cs.ns(|| "alloc y bit"), || {
            if point.y.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(point.y.get_value().get()?.into_repr() > half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Calculates and allocates the value y_adjusted.
        // This value is necessary so that later we can enforce the correctness of y_bit.
        let y_adjusted = FqGadget::alloc(cs.ns(|| "alloc y adjusted"), || {
            if point.y.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                let y_value = point.y.get_value().get()?;
                if y_value.into_repr() > half {
                    Ok(y_value - &(Fq::from_repr(half) + &Fq::one()))
                } else {
                    Ok(y_value)
                }
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Enforces the following relation:
        // y_adjusted <= half
        let y_adjusted_bits = &y_adjusted.to_bits(cs.ns(|| "y adjusted to bits"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, Fq, _>(
            cs.ns(|| "enforce y adjusted smaller than modulus minus one div two"),
            y_adjusted_bits,
            Fq::modulus_minus_one_div_two(),
        )?;

        // Enforces the following relation:
        // y + y_bit * (-(half + 1)) = y_adjusted
        let y_bit_lc = y_bit.lc(CS::one(), half_plus_one_neg);
        cs.enforce(
            || "check y bit",
            |lc| lc + (Fq::one(), CS::one()),
            |lc| point.y.get_variable() + y_bit_lc + lc,
            |lc| y_adjusted.get_variable() + lc,
        );

        Ok(y_bit)
    }

    /// Outputs a boolean representing the relation:
    /// (y_c2 > half) || (y_c2 == 0 && y_c1 > half) || (y_c2 == 0 && y_c1 == 0 && y_c0 > half)
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    pub fn y_to_bit_g2<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        point: &G2Gadget,
    ) -> Result<Boolean, SynthesisError> {
        // Calculates -(half + 1), where half = (p-1)/2.
        let half_plus_one_neg = (Fq::from_repr(Fq::modulus_minus_one_div_two()) + &Fq::one()).neg();

        // ----------------   y_c2 > half   ----------------
        // Calculates and allocates the bit representing the "sign" of the y-coordinate for this
        // corresponding field.
        let y_c2_bit = Boolean::alloc(cs.ns(|| "alloc y c2 bit"), || {
            if point.y.c2.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(point.y.c2.get_value().get()?.into_repr() > half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Calculates and allocates the value y_c2_adjusted.
        // This value is necessary so that later we can enforce the correctness of y_c2_bit.
        let y_c2_adjusted = FqGadget::alloc(cs.ns(|| "alloc y c2 adjusted"), || {
            if point.y.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                let y_value = point.y.c2.get_value().get()?;
                if y_value.into_repr() > half {
                    Ok(y_value - &(Fq::from_repr(half) + &Fq::one()))
                } else {
                    Ok(y_value)
                }
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Enforces the following relation:
        // y_c2_adjusted <= half
        let y_c2_adjusted_bits = &y_c2_adjusted.to_bits(cs.ns(|| "y c2 adjusted to bits"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, Fq, _>(
            cs.ns(|| "enforce y c2 adjusted smaller than modulus minus one div two"),
            y_c2_adjusted_bits,
            Fq::modulus_minus_one_div_two(),
        )?;

        // Enforces the following relation:
        // y_c2 + y_c2_bit * (-(half + 1)) = y_c2_adjusted
        let y_c2_bit_lc = y_c2_bit.lc(CS::one(), half_plus_one_neg);
        cs.enforce(
            || "check y c2 bit",
            |lc| lc + (Fq::one(), CS::one()),
            |lc| point.y.c2.get_variable() + y_c2_bit_lc + lc,
            |lc| y_c2_adjusted.get_variable() + lc,
        );

        // ----------------   y_c1 > half   ----------------
        // Calculates and allocates the bit representing the "sign" of the y-coordinate for this
        // corresponding field.
        let y_c1_bit = Boolean::alloc(cs.ns(|| "alloc y c1 bit"), || {
            if point.y.c1.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(point.y.c1.get_value().get()?.into_repr() > half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Calculates and allocates the value y_c1_adjusted.
        // This value is necessary so that later we can enforce the correctness of y_c1_bit.
        let y_c1_adjusted = FqGadget::alloc(cs.ns(|| "alloc y c1 adjusted"), || {
            if point.y.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                let y_value = point.y.c1.get_value().get()?;
                if y_value.into_repr() > half {
                    Ok(y_value - &(Fq::from_repr(half) + &Fq::one()))
                } else {
                    Ok(y_value)
                }
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Enforces the following relation:
        // y_c1_adjusted <= half
        let y_c1_adjusted_bits = &y_c1_adjusted.to_bits(cs.ns(|| "y c1 adjusted to bits"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, Fq, _>(
            cs.ns(|| "enforce y c1 adjusted smaller than modulus minus one div two"),
            y_c1_adjusted_bits,
            Fq::modulus_minus_one_div_two(),
        )?;

        // Enforces the following relation:
        // y_c1 + y_c1_bit * (-(half + 1)) = y_c1_adjusted
        let y_c1_bit_lc = y_c1_bit.lc(CS::one(), half_plus_one_neg);
        cs.enforce(
            || "check y c1 bit",
            |lc| lc + (Fq::one(), CS::one()),
            |lc| point.y.c1.get_variable() + y_c1_bit_lc + lc,
            |lc| y_c1_adjusted.get_variable() + lc,
        );

        // ----------------   y_c0 > half   ----------------
        // Calculates and allocates the bit representing the "sign" of the y-coordinate for this
        // corresponding field.
        let y_c0_bit = Boolean::alloc(cs.ns(|| "alloc y c0 bit"), || {
            if point.y.c0.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                Ok(point.y.c0.get_value().get()?.into_repr() > half)
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Calculates and allocates the value y_c0_adjusted.
        // This value is necessary so that later we can enforce the correctness of y_c0_bit.
        let y_c0_adjusted = FqGadget::alloc(cs.ns(|| "alloc y c0 adjusted"), || {
            if point.y.get_value().is_some() {
                let half = Fq::modulus_minus_one_div_two();
                let y_value = point.y.c0.get_value().get()?;
                if y_value.into_repr() > half {
                    Ok(y_value - &(Fq::from_repr(half) + &Fq::one()))
                } else {
                    Ok(y_value)
                }
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Enforces the following relation:
        // y_c0_adjusted <= half
        let y_c0_adjusted_bits = &y_c0_adjusted.to_bits(cs.ns(|| "y c0 adjusted to bits"))?;
        Boolean::enforce_smaller_or_equal_than::<_, _, Fq, _>(
            cs.ns(|| "enforce y c0 adjusted smaller than modulus minus one div two"),
            y_c0_adjusted_bits,
            Fq::modulus_minus_one_div_two(),
        )?;

        // Enforces the following relation:
        // y_c0 + y_c0_bit * (-(half + 1)) = y_c0_adjusted
        let y_c0_bit_lc = y_c0_bit.lc(CS::one(), half_plus_one_neg);
        cs.enforce(
            || "check y c0 bit",
            |lc| lc + (Fq::one(), CS::one()),
            |lc| point.y.c0.get_variable() + y_c0_bit_lc + lc,
            |lc| y_c0_adjusted.get_variable() + lc,
        );

        // ----------------   y_c2 == 0   ----------------
        // Calculates and allocates the bit representing if y_c2 == 0.
        let y_c2_eq_bit = Boolean::alloc(cs.ns(|| "alloc y c2 eq bit"), || {
            if point.y.c2.get_value().is_some() {
                Ok(point.y.c2.get_value().get()?.into_repr() == Fq::zero().into_repr())
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Calculates and allocates the inverse of y_c2.
        // This value is necessary so that later we can enforce the correctness of y_c2_eq_bit.
        let inv = FpGadget::alloc(cs.ns(|| "alloc y c2 inv"), || {
            if point.y.c2.get_value().is_some() {
                Ok(point
                    .y
                    .c2
                    .get_value()
                    .get()?
                    .inverse()
                    .unwrap_or_else(Fq::zero))
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Enforces the following relation:
        // y_c2 * y_c2_inv == 1 - y_c2_eq_bit
        // This guarantees that y_c2 and y_c2_eq_bit cannot both be zero.
        cs.enforce(
            || "enforce y_c2_eq_bit",
            |lc| point.y.c2.get_variable() + lc,
            |lc| inv.get_variable() + lc,
            |lc| lc + (Fq::one(), CS::one()) + y_c2_eq_bit.lc(CS::one(), Fq::one().neg()),
        );

        // Enforces the following relation:
        // y_c2 * y_c2_eq_bit == 0
        // This guarantees that y_c2 and y_c2_eq_bit cannot both be different from zero.
        cs.enforce(
            || "enforce y_c2_eq_bit 2",
            |lc| point.y.c2.get_variable() + lc,
            |_| y_c2_eq_bit.lc(CS::one(), Fq::one()),
            |lc| lc,
        );

        // ----------------   y_c1 == 0   ----------------
        // Calculates and allocates the bit representing if y_c1 == 0.
        let y_c1_eq_bit = Boolean::alloc(cs.ns(|| "alloc y c1 eq bit"), || {
            if point.y.c1.get_value().is_some() {
                Ok(point.y.c1.get_value().get()?.into_repr() == Fq::zero().into_repr())
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Calculates and allocates the inverse of y_c1.
        // This value is necessary so that later we can enforce the correctness of y_c1_eq_bit.
        let inv = FpGadget::alloc(cs.ns(|| "alloc y c1 inv"), || {
            if point.y.c1.get_value().is_some() {
                Ok(point
                    .y
                    .c1
                    .get_value()
                    .get()?
                    .inverse()
                    .unwrap_or_else(Fq::zero))
            } else {
                Err(SynthesisError::AssignmentMissing)
            }
        })?;

        // Enforces the following relation:
        // y_c1 * y_c1_inv == 1 - y_c1_eq_bit
        // This guarantees that y_c1 and y_c1_eq_bit cannot both be zero.
        cs.enforce(
            || "enforce y_c1_eq_bit",
            |lc| point.y.c1.get_variable() + lc,
            |lc| inv.get_variable() + lc,
            |lc| lc + (Fq::one(), CS::one()) + y_c1_eq_bit.lc(CS::one(), Fq::one().neg()),
        );

        // Enforces the following relation:
        // y_c1 * y_c1_eq_bit == 0
        // This guarantees that y_c1 and y_c1_eq_bit cannot both be different from zero.
        cs.enforce(
            || "enforce y_c1_eq_bit 2",
            |lc| point.y.c1.get_variable() + lc,
            |_| y_c1_eq_bit.lc(CS::one(), Fq::one()),
            |lc| lc,
        );

        // --------  (y_c2 > half) || (y_c2 == 0 && y_c1 > half) || (y_c2 == 0 && y_c1 == 0 && y_c0 > half)  --------
        let cond0 = y_c2_bit;
        let cond1 = Boolean::and(cs.ns(|| "y_c2_eq_bit && y_c1_bit"), &y_c2_eq_bit, &y_c1_bit)?;
        let cond2 = Boolean::kary_and(
            cs.ns(|| "y_c2_eq_bit && y_c1_eq_bit && y_c0_bit"),
            vec![y_c2_eq_bit, y_c1_eq_bit, y_c0_bit].as_ref(),
        )?;
        let y_bit = Boolean::kary_or(
            cs.ns(|| "cond0 || cond1 || cond2"),
            vec![cond0, cond1, cond2].as_ref(),
        )?;

        Ok(y_bit)
    }
}
