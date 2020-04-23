use std::marker::PhantomData;

use algebra_core::{PrimeField, SWModelParameters};
use r1cs_core::SynthesisError;
use r1cs_std::boolean::Boolean;
use r1cs_std::fields::FieldGadget;
use r1cs_std::groups::curves::short_weierstrass::AffineGadget;
use r1cs_std::groups::GroupGadget;
use r1cs_std::prelude::CondSelectGadget;

/// This is a gadget that calculates a Pedersen hash. It is collision resistant, but it's not
/// pseudo-random. Furthermore, its input must have a fixed-length. The main advantage is that it
/// is purely algebraic and its output is an elliptic curve point.
/// The Pedersen hash guarantees that the exponent of the resulting point H is not known, the same
/// can't be said of the Pedersen commitment. Also, note that the Pedersen hash is collision-resistant
/// but it is not pseudo-random.
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
    /// Calculates the Pedersen hash. Given a vector of generators G_i and a vector of bits b_i, the
    /// hash is calculated like so:
    /// H = G_0 + b_1 * G_1 + b_2 * G_2 + ... + b_n * G_n
    /// where G_0 is a sum generator that is used to avoid that the sum starts at zero (which is
    /// problematic because the circuit can't handle addition with zero).
    pub fn evaluate<CS: r1cs_core::ConstraintSystem<ConstraintF>>(
        mut cs: CS,
        generators: &Vec<AffineGadget<P, ConstraintF, F>>,
        input: &Vec<Boolean>,
        sum_generator: &AffineGadget<P, ConstraintF, F>,
    ) -> Result<AffineGadget<P, ConstraintF, F>, SynthesisError> {
        // Verify that we have enough generators for the input bits.
        assert!(generators.len() >= input.len());

        // Initiate the result with the sum generator. We can't initiate it with the neutral element
        // because that would result in an error. The addition function for EC points is incomplete
        // and can't handle the neutral element (aka zero, point-at-infinity).
        // This weird initialization (instead of just doing let mut result = sum_generator) is to
        // appease Rust's borrow checker. Best to leave it as is!
        let mut result = AffineGadget::<P, ConstraintF, F>::new(
            sum_generator.x.clone(),
            sum_generator.y.clone(),
            sum_generator.infinity,
        );

        for i in 0..input.len() {
            // Add the next generator to the current sum.
            let new_sum = result.add(cs.ns(|| format!("add bit {}", i)), &generators[i])?;
            // If the bit is zero, keep the current sum. If it is one, take the new sum.
            result = AffineGadget::<P, ConstraintF, F>::conditionally_select(
                &mut cs.ns(|| format!("Conditional Select {}", i)),
                &input[i],
                &new_sum,
                &result,
            )?;
        }

        Ok(result)
    }
}

/// This is a gadget that calculates a Pedersen commitment. The main advantage is that it is purely
/// algebraic and its output is an elliptic curve point.
/// The Pedersen commitment takes the same time/constraints to calculate as the Pedersen hash but,
/// since it requires fewer generators, it is faster to setup.
pub struct PedersenCommitmentGadget<
    P: SWModelParameters,
    ConstraintF: PrimeField,
    F: FieldGadget<P::BaseField, ConstraintF>,
> {
    _params: PhantomData<P>,
    _cfield: PhantomData<ConstraintF>,
    _field: PhantomData<F>,
}

impl<P: SWModelParameters, ConstraintF: PrimeField, F: FieldGadget<P::BaseField, ConstraintF>>
    PedersenCommitmentGadget<P, ConstraintF, F>
{
    /// Calculates the Pedersen commitment. Given a vector of bits b_i we divide the vector into chunks
    /// of 752 bits and convert them into scalars like so:
    /// s = b_0 * 2^0 + b_1 * 2^1 + ... + b_750 * 2^750 + b_751 * 2^751
    /// We then calculate the commitment like so:
    /// C = G_0 + s_1 * G_1 + ... + s_n * G_n
    /// where G_0 is a sum generator that is used to avoid that the sum starts at zero (which is
    /// problematic because the circuit can't handle addition with zero).
    pub fn evaluate<CS: r1cs_core::ConstraintSystem<ConstraintF>>(
        mut cs: CS,
        generators: &Vec<AffineGadget<P, ConstraintF, F>>,
        input: &Vec<Boolean>,
        sum_generator: &AffineGadget<P, ConstraintF, F>,
    ) -> Result<AffineGadget<P, ConstraintF, F>, SynthesisError> {
        // This is simply the number of bits that each generator can store.
        let capacity = 752;

        // Check that the input can be stored using the available generators.
        assert!(generators.len() * capacity >= input.len());

        // Calculate the rounds that are necessary to process the input.
        let normal_rounds = input.len() / capacity;

        // Initiate the result with the sum generator. We can't initiate it with the neutral element
        // because that would result in an error. The addition function for EC points is incomplete
        // and can't handle the neutral element (aka zero, point-at-infinity).
        // This weird initialization (instead of just doing let mut result = sum_generator) is to
        // appease Rust's borrow checker. Best to leave it as is!
        let mut result = AffineGadget::<P, ConstraintF, F>::new(
            sum_generator.x.clone(),
            sum_generator.y.clone(),
            sum_generator.infinity,
        );

        // Start calculating the Pedersen commitment. We use the double-and-add method for EC point
        // multiplication for each generator.
        for i in 0..normal_rounds {
            result = generators[i].mul_bits(
                cs.ns(|| format!("double-and-add generator {}", i)),
                &result,
                input[i * capacity..(i + 1) * capacity].iter(),
            )?;
        }

        // Begin the final point multiplication. For this one we don't use all 752 bits.
        result = generators[normal_rounds].mul_bits(
            cs.ns(|| format!("double-and-add generator {}", normal_rounds)),
            &result,
            input[normal_rounds * capacity..].iter(),
        )?;

        Ok(result)
    }
}
