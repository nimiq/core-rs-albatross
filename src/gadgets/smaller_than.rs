use algebra::{Field, PrimeField};
use r1cs_core::{ConstraintSystem, SynthesisError};
use r1cs_std::{
    boolean::Boolean,
    fields::{fp::FpGadget, FieldGadget},
    ToBitsGadget,
};
use std::marker::PhantomData;

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

#[cfg(test)]
mod test {
    use rand::{Rng, SeedableRng};
    use rand_xorshift::XorShiftRng;

    use super::SmallerThanGadget;
    use algebra::{
        curves::{
            bls12_377::{
                Bls12_377, G1Projective as Bls12_377G1Projective,
                G2Projective as Bls12_377G2Projective,
            },
            ProjectiveCurve,
        },
        fields::bls12_377::Fr as Bls12_377Fr,
        fields::sw6::Fr as SW6Fr,
        fields::{Field, PrimeField},
        UniformRand,
    };
    use r1cs_core::ConstraintSystem;
    use r1cs_std::fields::fp::FpGadget;
    use r1cs_std::{
        alloc::AllocGadget,
        boolean::Boolean,
        groups::bls12::bls12_377::{G1Gadget as Bls12_377G1Gadget, G2Gadget as Bls12_377G2Gadget},
        pairing::bls12_377::PairingGadget as Bls12_377PairingGadget,
        test_constraint_system::TestConstraintSystem,
    };

    #[test]
    fn test_smaller_than() {
        let mut rng = &mut XorShiftRng::from_seed([
            0x5d, 0xbe, 0x62, 0x59, 0x8d, 0x31, 0x3d, 0x76, 0x32, 0x37, 0xdb, 0x17, 0xe5, 0xbc,
            0x06, 0x54,
        ]);
        fn rand_in_range<R: Rng>(rng: &mut R) -> SW6Fr {
            let pminusonedivtwo = SW6Fr::from_repr(SW6Fr::modulus_minus_one_div_two());
            let mut r = SW6Fr::zero();
            loop {
                r = SW6Fr::rand(rng);
                if r <= pminusonedivtwo {
                    break;
                }
            }
            r
        }
        for i in 0..10 {
            let mut cs = TestConstraintSystem::<SW6Fr>::new();
            let a = rand_in_range(&mut rng);
            let a_var = FpGadget::<SW6Fr>::alloc(cs.ns(|| "a"), || Ok(a)).unwrap();
            let b = rand_in_range(&mut rng);
            let b_var = FpGadget::<SW6Fr>::alloc(cs.ns(|| "b"), || Ok(b)).unwrap();

            if a < b {
                SmallerThanGadget::<SW6Fr>::enforce_smaller_than_strict(
                    cs.ns(|| "smaller than test"),
                    &a_var,
                    &b_var,
                )
                .unwrap();
            } else {
                SmallerThanGadget::<SW6Fr>::enforce_smaller_than_strict(
                    cs.ns(|| "smaller than test"),
                    &b_var,
                    &a_var,
                )
                .unwrap();
            }

            if i == 0 {
                println!("number of constraints: {}", cs.num_constraints());
            }
            assert!(cs.is_satisfied());
        }

        for _i in 0..10 {
            let mut cs = TestConstraintSystem::<SW6Fr>::new();
            let a = rand_in_range(&mut rng);
            let a_var = FpGadget::<SW6Fr>::alloc(cs.ns(|| "a"), || Ok(a)).unwrap();
            let b = rand_in_range(&mut rng);
            let b_var = FpGadget::<SW6Fr>::alloc(cs.ns(|| "b"), || Ok(b)).unwrap();

            if b < a {
                SmallerThanGadget::<SW6Fr>::enforce_smaller_than_strict(
                    cs.ns(|| "smaller than test"),
                    &a_var,
                    &b_var,
                )
                .unwrap();
            } else {
                SmallerThanGadget::<SW6Fr>::enforce_smaller_than_strict(
                    cs.ns(|| "smaller than test"),
                    &b_var,
                    &a_var,
                )
                .unwrap();
            }

            assert!(!cs.is_satisfied());
        }

        for _i in 0..10 {
            let mut cs = TestConstraintSystem::<SW6Fr>::new();
            let a = rand_in_range(&mut rng);
            let a_var = FpGadget::<SW6Fr>::alloc(cs.ns(|| "a"), || Ok(a)).unwrap();
            SmallerThanGadget::<SW6Fr>::enforce_smaller_than_strict(
                cs.ns(|| "smaller than test"),
                &a_var,
                &a_var,
            )
            .unwrap();

            assert!(!cs.is_satisfied());
        }
    }
}
