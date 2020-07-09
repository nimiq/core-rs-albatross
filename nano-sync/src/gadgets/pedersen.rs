use std::marker::PhantomData;

use algebra_core::{PrimeField, SWModelParameters};
use r1cs_core::SynthesisError;
use r1cs_std::boolean::Boolean;
use r1cs_std::fields::FieldGadget;
use r1cs_std::groups::curves::short_weierstrass::AffineGadget;
use r1cs_std::groups::GroupGadget;

use crate::constants::POINT_CAPACITY;

/// This is a gadget that calculates a Pedersen hash. It is collision resistant, but it's not
/// pseudo-random. Furthermore, its input must have a fixed-length. The main advantage is that it
/// is purely algebraic and its output is an elliptic curve point.
/// The Pedersen hash also guarantees that the exponent of the resulting point H is not known.
pub struct PedersenHashGadget<
    P: SWModelParameters,
    ConstraintF: PrimeField,
    F: FieldGadget<P::BaseField, ConstraintF>,
> {
    _params: PhantomData<P>,
    _cfield: PhantomData<ConstraintF>,
    _field: PhantomData<F>,
}

impl<P: SWModelParameters, ConstraintF: PrimeField, F: FieldGadget<P::BaseField, ConstraintF>>
    PedersenHashGadget<P, ConstraintF, F>
{
    /// Calculates the Pedersen hash. Given a vector of bits b_i we divide the vector into chunks
    /// of 752 bits (because that's the capacity of the MNT curves) and convert them into scalars
    /// like so:
    /// s = b_0 * 2^0 + b_1 * 2^1 + ... + b_750 * 2^750 + b_751 * 2^751
    /// We then calculate the hash like so:
    /// H = G_0 + s_1 * G_1 + ... + s_n * G_n
    /// where G_0 is a sum generator that is used to avoid that the sum starts at zero (which is
    /// problematic because the circuit can't handle addition with zero) and to guarantee that the
    /// exponent of the resulting EC point is not known (necessary for BLS signatures).
    pub fn evaluate<CS: r1cs_core::ConstraintSystem<ConstraintF>>(
        mut cs: CS,
        input: &Vec<Boolean>,
        generators: &Vec<AffineGadget<P, ConstraintF, F>>,
    ) -> Result<AffineGadget<P, ConstraintF, F>, SynthesisError> {
        // Check that the input can be stored using the available generators.
        assert!((generators.len() - 1) * POINT_CAPACITY >= input.len());

        // Calculate the rounds that are necessary to process the input.
        let normal_rounds = input.len() / POINT_CAPACITY;

        // Initiate the result with the sum generator. We can't initiate it with the neutral element
        // because that would result in an error. The addition function for EC points is incomplete
        // and can't handle the neutral element (aka zero, point-at-infinity).
        let mut result = generators[0].clone();

        // Start calculating the Pedersen hash. We use the double-and-add method for EC point
        // multiplication for each generator.
        for i in 0..normal_rounds {
            result = generators[i + 1].mul_bits(
                cs.ns(|| format!("double-and-add generator {}", i)),
                &result,
                input[i * POINT_CAPACITY..(i + 1) * POINT_CAPACITY].iter(),
            )?;
        }

        // Begin the final point multiplication. For this one we don't use all 752 bits.
        result = generators[normal_rounds + 1].mul_bits(
            cs.ns(|| format!("double-and-add generator {}", normal_rounds)),
            &result,
            input[normal_rounds * POINT_CAPACITY..].iter(),
        )?;

        Ok(result)
    }
}
