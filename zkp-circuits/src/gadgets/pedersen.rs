use std::borrow::Borrow;

use ark_crypto_primitives::crh::{
    pedersen::{
        constraints::{CRHGadget, CRHParametersVar},
        Window,
    },
    CRHSchemeGadget,
};
use ark_ec::CurveGroup;
use ark_ff::Field;
use ark_r1cs_std::{
    prelude::{AllocVar, AllocationMode, CurveVar, GroupOpsBounds},
    uint8::UInt8,
};
use ark_relations::r1cs::{Namespace, SynthesisError};
use nimiq_pedersen_generators::PedersenParameters;

pub struct PedersenParametersVar<C: CurveGroup, GG: CurveVar<C, ConstraintF<C>>>
where
    for<'a> &'a GG: GroupOpsBounds<'a, C, GG>,
{
    parameters: CRHParametersVar<C, GG>,
    blinding_factor: GG,
}

impl<C, GG> AllocVar<PedersenParameters<C>, ConstraintF<C>> for PedersenParametersVar<C, GG>
where
    C: CurveGroup,
    GG: CurveVar<C, ConstraintF<C>>,
    for<'a> &'a GG: GroupOpsBounds<'a, C, GG>,
{
    fn new_variable<T: Borrow<PedersenParameters<C>>>(
        cs: impl Into<Namespace<ConstraintF<C>>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
        mode: AllocationMode,
    ) -> Result<Self, SynthesisError> {
        let cs = cs.into();
        let value = f()?;
        let params = &(*value.borrow()).clone();
        Ok(PedersenParametersVar {
            parameters: CRHParametersVar::new_variable(
                cs.clone(),
                || Ok(&params.parameters),
                mode,
            )?,
            blinding_factor: <GG as AllocVar<C, _>>::new_variable(
                cs,
                || Ok(&params.blinding_factor),
                mode,
            )?,
        })
    }
}

/// This is a gadget that calculates a Pedersen hash. It is collision resistant, but it's not
/// pseudo-random. Furthermore, its input must have a fixed-length. The main advantage is that it
/// is purely algebraic and its output is an elliptic curve point.
/// The Pedersen hash also guarantees that the exponent of the resulting point H is not known.
type ConstraintF<C> = <<C as CurveGroup>::BaseField as Field>::BasePrimeField;
pub struct PedersenHashGadget<C: CurveGroup, GG: CurveVar<C, ConstraintF<C>>, W: Window>
where
    for<'a> &'a GG: GroupOpsBounds<'a, C, GG>,
{
    _gadget: CRHGadget<C, GG, W>,
}

impl<C: CurveGroup, GG: CurveVar<C, ConstraintF<C>>, W: Window> PedersenHashGadget<C, GG, W>
where
    for<'a> &'a GG: GroupOpsBounds<'a, C, GG>,
{
    /// Calculates the Pedersen hash. Given a vector of bits b_i we divide the vector into chunks
    /// of 752 bits (because that's the capacity of the MNT curves) and convert them into scalars
    /// like so:
    /// s = b_0 * 2^0 + b_1 * 2^1 + ... + b_750 * 2^750 + b_751 * 2^751
    /// We then calculate the hash like so:
    /// H = G_0 + s_1 * G_1 + ... + s_n * G_n
    /// where G_0 is a sum generator that is used to guarantee that the exponent of the resulting
    /// EC point is not known (necessary for BLS signatures).
    pub fn evaluate(
        input: &[UInt8<ConstraintF<C>>],
        params: &PedersenParametersVar<C, GG>,
    ) -> Result<GG, SynthesisError> {
        let mut hash = CRHGadget::<C, GG, W>::evaluate(&params.parameters, input)?;
        hash += &params.blinding_factor;
        Ok(hash)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt6_753::{constraints::G1Var, Fq as MNT6Fq, G1Projective, MNT6_753};
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::test_rng;
    use nimiq_pedersen_generators::{pedersen_generator_powers, GenericWindow};
    use nimiq_test_log::test;
    use nimiq_zkp_primitives::pedersen::pedersen_hash;
    use rand::RngCore;

    use super::*;

    #[test]
    fn pedersen_hash_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random bytes.
        let mut bytes = [0u8; 450];
        rng.fill_bytes(&mut bytes);

        // Generate the generators for the Pedersen hash.
        let parameters = pedersen_generator_powers::<GenericWindow<5, MNT6Fq>, MNT6_753>(1);

        // Evaluate Pedersen hash using the primitive version.
        let primitive_hash = pedersen_hash::<_, GenericWindow<5, MNT6Fq>>(&bytes, &parameters);

        // Allocate the random bits in the circuit.
        let bytes_var = UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &bytes).unwrap();

        // Allocate the Pedersen generators in the circuit.
        let generators_var =
            PedersenParametersVar::<G1Projective, G1Var>::new_witness(cs, || Ok(&parameters))
                .unwrap();

        // Evaluate Pedersen hash using the gadget version.
        let gadget_hash = PedersenHashGadget::<_, _, GenericWindow<5, MNT6Fq>>::evaluate(
            &bytes_var,
            &generators_var,
        )
        .unwrap();

        assert_eq!(primitive_hash, gadget_hash.value().unwrap())
    }
}
