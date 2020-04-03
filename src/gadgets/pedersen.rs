use std::borrow::Borrow;

use algebra::sw6::Fr as SW6Fr;
use r1cs_core::SynthesisError;
use r1cs_std::bits::uint8::UInt8;
use r1cs_std::bls12_377::G1Gadget;
use r1cs_std::boolean::Boolean;
use r1cs_std::groups::GroupGadget;
use r1cs_std::prelude::CondSelectGadget;
use r1cs_std::ToBitsGadget;

/// This is a gadget that calculates a Pedersen hash. It is a collision resistant, but it's not
/// pseudo-random. Furthermore, its input must have a fixed-length. The main advantage is that it
/// is purely algebraic and its output is an elliptic curve point.  
pub struct PedersenHashGadget;

impl PedersenHashGadget {
    /// Calculates the Pedersen hash. Given a vector of generators G_i and a vector of bits b_i, the
    /// hash is calculated like so:
    /// H = b_1*G_1 + b_2*G_2 + ... + b_n*G_n
    pub fn evaluate<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        generators: &Vec<G1Gadget>,
        input: &Vec<UInt8>,
        sum_generator: &G1Gadget,
    ) -> Result<G1Gadget, SynthesisError> {
        // Initialize Boolean vector.
        let mut bits: Vec<Boolean> = vec![];

        // Convert each byte of the input to bits and append it.
        for i in 0..input.len() {
            let chunk = input[i].to_bits(&mut cs.ns(|| format!("to bits {}", i)))?;
            // Append to Boolean vector.
            bits.extend(chunk);
        }

        // Initiate the result with the sum generator. We can't initiate it with the neutral element
        // because that would result in an error. The addition function for EC points is incomplete
        // and can't handle the neutral element (aka zero, point-at-infinity).
        let mut result = sum_generator;
        let mut new_result;

        for i in 0..bits.len() {
            // Add the next generator to the current sum.
            let new_sum = result.add(cs.ns(|| format!("add bit {}", i)), &generators[i])?;
            // If the bit is zero, keep the current sum. If it is one, take the new sum.
            new_result = G1Gadget::conditionally_select(
                &mut cs.ns(|| format!("Conditional Select {}", i)),
                bits[i].borrow(),
                &new_sum,
                &result,
            )?;
            // This whole thing with the result and new_result is needed because the Rust borrow checker
            // complains otherwise. Looks weird but better to leave it as is.
            result = &new_result;
        }

        // Finally subtract the sum generator from the current sum. In the end, this gives the same
        // result as if we just started the sum with zero.
        let result = result.sub(cs.ns(|| "sub generator"), sum_generator)?;

        Ok(result)
    }
}
