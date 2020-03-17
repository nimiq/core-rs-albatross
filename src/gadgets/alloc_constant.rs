use algebra::curves::models::short_weierstrass_jacobian::{GroupAffine, GroupProjective};
use algebra::{Field, Fp2, Fp2Parameters, PrimeField, ProjectiveCurve, SWModelParameters};
use r1cs_core::{ConstraintSystem, SynthesisError};
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::fields::{fp::FpGadget, fp2::Fp2Gadget};
use r1cs_std::groups::curves::short_weierstrass::AffineGadget;
use r1cs_std::prelude::FieldGadget;

pub trait AllocConstantGadget<V, ConstraintF: Field>
where
    Self: Sized,
    V: ?Sized,
{
    fn alloc_const<CS: ConstraintSystem<ConstraintF>>(
        cs: CS,
        constant: &V,
    ) -> Result<Self, SynthesisError>;
}

impl<I, ConstraintF: Field, A: AllocConstantGadget<I, ConstraintF>>
    AllocConstantGadget<[I], ConstraintF> for Vec<A>
{
    fn alloc_const<CS: ConstraintSystem<ConstraintF>>(
        mut cs: CS,
        constant: &[I],
    ) -> Result<Self, SynthesisError> {
        let mut vec = Vec::new();
        for (i, value) in constant.iter().enumerate() {
            vec.push(A::alloc_const(cs.ns(|| format!("value_{}", i)), value)?);
        }
        Ok(vec)
    }
}

impl<F: PrimeField> AllocConstantGadget<F, F> for FpGadget<F> {
    fn alloc_const<CS: ConstraintSystem<F>>(
        mut cs: CS,
        constant: &F,
    ) -> Result<Self, SynthesisError> {
        let mut value = FpGadget::one(cs.ns(|| "alloc one"))?;
        value.mul_by_constant_in_place(cs.ns(|| "mul by const"), constant)?;
        Ok(value)
    }
}

impl<P: Fp2Parameters<Fp = ConstraintF>, ConstraintF: PrimeField>
    AllocConstantGadget<Fp2<P>, ConstraintF> for Fp2Gadget<P, ConstraintF>
{
    fn alloc_const<CS: ConstraintSystem<ConstraintF>>(
        mut cs: CS,
        constant: &Fp2<P>,
    ) -> Result<Self, SynthesisError> {
        let c0 = AllocConstantGadget::alloc_const(cs.ns(|| "c0"), &constant.c0)?;
        let c1 = AllocConstantGadget::alloc_const(cs.ns(|| "c1"), &constant.c1)?;
        let value = Fp2Gadget::new(c0, c1);
        Ok(value)
    }
}

impl<P, ConstraintF, F> AllocConstantGadget<GroupProjective<P>, ConstraintF>
    for AffineGadget<P, ConstraintF, F>
where
    P: SWModelParameters,
    ConstraintF: PrimeField,
    F: FieldGadget<P::BaseField, ConstraintF> + AllocConstantGadget<P::BaseField, ConstraintF>,
{
    fn alloc_const<CS: ConstraintSystem<ConstraintF>>(
        cs: CS,
        constant: &GroupProjective<P>,
    ) -> Result<Self, SynthesisError> {
        let affine = constant.into_affine();
        AllocConstantGadget::alloc_const(cs, &affine)
    }
}

impl<P, ConstraintF, F> AllocConstantGadget<GroupAffine<P>, ConstraintF>
    for AffineGadget<P, ConstraintF, F>
where
    P: SWModelParameters,
    ConstraintF: PrimeField,
    F: FieldGadget<P::BaseField, ConstraintF> + AllocConstantGadget<P::BaseField, ConstraintF>,
{
    fn alloc_const<CS: ConstraintSystem<ConstraintF>>(
        mut cs: CS,
        constant: &GroupAffine<P>,
    ) -> Result<Self, SynthesisError> {
        let x = AllocConstantGadget::alloc_const(cs.ns(|| "x coordinate"), &constant.x)?;
        let y = AllocConstantGadget::alloc_const(cs.ns(|| "y coordinate"), &constant.y)?;
        let infinity = Boolean::constant(constant.infinity);
        Ok(AffineGadget::new(x, y, infinity))
    }
}
