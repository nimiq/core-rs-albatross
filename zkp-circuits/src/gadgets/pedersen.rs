use std::borrow::Borrow;

use ark_crypto_primitives::crh::pedersen::{constraints::CRHGadget, Parameters, Window};
use ark_ec::CurveGroup;
use ark_ff::Field;
use ark_r1cs_std::{
    prelude::{AllocVar, AllocationMode, CurveVar, GroupOpsBounds},
    uint8::UInt8,
    ToBitsGadget,
};
use ark_relations::r1cs::{Namespace, SynthesisError};
use nimiq_pedersen_generators::generators::PedersenParameters;

pub struct PedersenParametersVar<C: CurveGroup, GG: CurveVar<C, ConstraintF<C>>>
where
    for<'a> &'a GG: GroupOpsBounds<'a, C, GG>,
{
    parameters: Parameters<C>, //CRHParametersVar<C, GG>,
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
            // parameters: CRHParametersVar::new_variable(
            //     cs.clone(),
            //     || Ok(&params.parameters),
            //     mode,
            // )?,
            parameters: params.parameters.clone(),
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
        // There's a mismatch between the on-circuit and off-circuit hashes when using the arkworks implementation.
        // Thus we resort to our own and will change back once in the future.
        // let mut hash = CRHGadget::<C, GG, W>::evaluate(&params.parameters, input)?;
        // hash += &params.blinding_factor;
        // Ok(hash)

        // Check that the input can be stored using the available generators.
        assert!(params.parameters.generators.len() >= W::NUM_WINDOWS);
        assert!(W::NUM_WINDOWS * W::WINDOW_SIZE >= input.len() / 8);

        let bits = input.to_bits_le()?;

        // Initiate the result with the sum generator.
        let mut result = params.blinding_factor.clone();

        // Start calculating the Pedersen hash. We use the double-and-add method for EC point
        // multiplication for each generator.
        for (i, chunk) in bits.chunks(W::WINDOW_SIZE).enumerate() {
            for (j, bit) in chunk.iter().enumerate() {
                // Add the base to the result.
                let self_plus_base = result.clone() + params.parameters.generators[i][j];

                // Depending on the bit, either select the new result (with the base added) or
                // continue with the previous result.
                result = GG::conditionally_select(bit, &self_plus_base, &result)?;
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt6_753::{constraints::G1Var, Fq as MNT6Fq, G1Projective};
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::test_rng;
    use nimiq_zkp_primitives::pedersen::pedersen_hash;
    use rand::RngCore;

    use nimiq_pedersen_generators::{generators::pedersen_generator_powers, GenericWindow};
    use nimiq_test_log::test;

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
        let parameters = pedersen_generator_powers::<GenericWindow<5, MNT6Fq>>();

        // Evaluate Pedersen hash using the primitive version.
        let primitive_hash = pedersen_hash::<_, GenericWindow<5, MNT6Fq>>(&bytes, &parameters);

        // Allocate the random bits in the circuit.
        let bytes_var = UInt8::<MNT6Fq>::new_witness_vec(cs.clone(), &bytes).unwrap();

        // Allocate the Pedersen generators in the circuit.
        let generators_var =
            PedersenParametersVar::<G1Projective, G1Var>::new_witness(cs.clone(), || {
                Ok(&parameters)
            })
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
