use std::{borrow::Borrow, ops::Deref};

use ark_ec::short_weierstrass::{Affine, SWCurveConfig, SWFlags};
use ark_ff::Field;
use ark_r1cs_std::{
    alloc::{AllocVar, AllocationMode},
    boolean::Boolean,
    eq::EqGadget,
    fields::{FieldOpsBounds, FieldVar},
    groups::curves::short_weierstrass::{
        non_zero_affine::NonZeroAffineVar, AffineVar, ProjectiveVar,
    },
};
use ark_relations::r1cs::{Namespace, SynthesisError};

use super::y_to_bit::YToBitGadget;

pub struct CompressedAffineVar<
    P: SWCurveConfig,
    F: FieldVar<P::BaseField, <P::BaseField as Field>::BasePrimeField>,
> where
    for<'a> &'a F: FieldOpsBounds<'a, P::BaseField, F>,
{
    pub affine: AffineVar<P, F>,
}

impl<P: SWCurveConfig, F: FieldVar<P::BaseField, <P::BaseField as Field>::BasePrimeField>> Deref
    for CompressedAffineVar<P, F>
where
    for<'a> &'a F: FieldOpsBounds<'a, P::BaseField, F>,
{
    type Target = AffineVar<P, F>;

    fn deref(&self) -> &Self::Target {
        &self.affine
    }
}

impl<P: SWCurveConfig, F: FieldVar<P::BaseField, <P::BaseField as Field>::BasePrimeField>>
    CompressedAffineVar<P, F>
where
    for<'a> &'a F: FieldOpsBounds<'a, P::BaseField, F>,
    AffineVar<P, F>: YToBitGadget<<P::BaseField as Field>::BasePrimeField>,
{
    /// This function allows passing the y-bit separately, so we can unpack it outside
    /// and use a space efficient encoding.
    pub fn with_y_bit<T: Borrow<Affine<P>>>(
        cs: impl Into<Namespace<<P::BaseField as Field>::BasePrimeField>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        y_is_negative_var: &Boolean<<P::BaseField as Field>::BasePrimeField>,
    ) -> Result<Self, SynthesisError> {
        let ns = cs.into();
        let cs = ns.cs();
        let (value, x_coord) = match f() {
            Ok(v) => {
                let value = v.borrow();
                let x = value.x;
                (Ok(v), Ok(x))
            }
            Err(e) => (Err(e), Err(e)),
        };

        // Allocate as witness.
        let var = Self::new_witness(cs.clone(), || value)?;

        // Only get x and negative bit as input (we pass the negative bit separately for optimization).
        let x_var = F::new_input(cs.clone(), || x_coord)?;

        // Then check they correspond.
        var.affine.x.enforce_equal(&x_var)?;

        let y_bit = var.affine.y_to_bit(cs)?;
        y_bit.enforce_equal(y_is_negative_var)?;
        var.affine.infinity.enforce_equal(&Boolean::FALSE)?;

        Ok(var)
    }

    pub fn into_projective(self) -> ProjectiveVar<P, F> {
        // We checked that this is not zero at allocation.
        let non_zero = NonZeroAffineVar::new(self.affine.x, self.affine.y);
        non_zero.into_projective()
    }
}

impl<P: SWCurveConfig, F: FieldVar<P::BaseField, <P::BaseField as Field>::BasePrimeField>>
    AllocVar<Affine<P>, <P::BaseField as Field>::BasePrimeField> for CompressedAffineVar<P, F>
where
    for<'a> &'a F: FieldOpsBounds<'a, P::BaseField, F>,
    AffineVar<P, F>: YToBitGadget<<P::BaseField as Field>::BasePrimeField>,
{
    fn new_variable<T: Borrow<Affine<P>>>(
        cs: impl Into<Namespace<<P::BaseField as Field>::BasePrimeField>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        mode: AllocationMode,
    ) -> Result<Self, SynthesisError> {
        Ok(Self {
            affine: ProjectiveVar::new_variable(cs, f, mode)?.to_affine()?,
        })
    }

    fn new_input<T: Borrow<Affine<P>>>(
        cs: impl Into<Namespace<<P::BaseField as Field>::BasePrimeField>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
    ) -> Result<Self, SynthesisError> {
        let ns = cs.into();
        let cs = ns.cs();
        let (value, y_is_negative) = match f() {
            Ok(v) => {
                let value = v.borrow();
                let is_neg = !SWFlags::from_y_coordinate(value.y)
                    .is_positive()
                    .ok_or(SynthesisError::AssignmentMissing)?;
                (Ok(v), Ok(is_neg))
            }
            Err(e) => (Err(e), Err(e)),
        };

        // Only negative bit as input.
        let is_negative_var = Boolean::new_input(cs.clone(), || y_is_negative)?;

        Self::with_y_bit(cs, || value, &is_negative_var)
    }
}

#[cfg(test)]
mod mnt6_tests {
    use ark_mnt6_753::{
        g1::Config as G1Config, g2::Config as G2Config, Fq as MNT6Fq, G1Affine, G2Affine,
    };
    use ark_r1cs_std::prelude::AllocVar;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn compressed_affine_g1_mnt6_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        for _ in 0..10 {
            // Create random point.
            let g1_point = G1Affine::rand(rng);

            // Allocate the random inputs in the circuit and convert it to affine form.
            CompressedAffineVar::<G1Config, _>::new_input(cs.clone(), || Ok(g1_point)).unwrap();
        }

        assert!(cs.is_satisfied().unwrap());
    }

    #[test]
    fn compressed_affine_g2_mnt6_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        for _ in 0..10 {
            // Create random point.
            let g2_point = G2Affine::rand(rng);

            // Allocate the random inputs in the circuit and convert it to affine form.
            CompressedAffineVar::<G2Config, _>::new_input(cs.clone(), || Ok(g2_point)).unwrap();
        }

        assert!(cs.is_satisfied().unwrap());
    }
}

#[cfg(test)]
mod mnt4_tests {
    use ark_mnt4_753::{
        g1::Config as G1Config, g2::Config as G2Config, Fq as MNT4Fq, G1Affine, G2Affine,
    };
    use ark_r1cs_std::prelude::AllocVar;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn compressed_affine_g1_mnt4_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        for _ in 0..10 {
            // Create random point.
            let g1_point = G1Affine::rand(rng);

            // Allocate the random inputs in the circuit and convert it to affine form.
            CompressedAffineVar::<G1Config, _>::new_input(cs.clone(), || Ok(g1_point)).unwrap();
        }

        assert!(cs.is_satisfied().unwrap());
    }

    #[test]
    fn compressed_affine_g2_mnt4_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        for _ in 0..10 {
            // Create random point.
            let g2_point = G2Affine::rand(rng);

            // Allocate the random inputs in the circuit and convert it to affine form.
            CompressedAffineVar::<G2Config, _>::new_input(cs.clone(), || Ok(g2_point)).unwrap();
        }

        assert!(cs.is_satisfied().unwrap());
    }
}
