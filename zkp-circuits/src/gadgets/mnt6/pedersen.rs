use ark_mnt6_753::{constraints::G1Var, Fq as MNT6Fq};
use ark_r1cs_std::prelude::{Boolean, CondSelectGadget, CurveVar};
use ark_relations::r1cs::SynthesisError;

/// This is a gadget that calculates a Pedersen hash. It is collision resistant, but it's not
/// pseudo-random. Furthermore, its input must have a fixed-length. The main advantage is that it
/// is purely algebraic and its output is an elliptic curve point.
/// The Pedersen hash also guarantees that the exponent of the resulting point H is not known.
pub struct PedersenHashGadget {}

impl PedersenHashGadget {
    /// Calculates the Pedersen hash. Given a vector of bits b_i we divide the vector into chunks
    /// of 752 bits (because that's the capacity of the MNT curves) and convert them into scalars
    /// like so:
    /// s = b_0 * 2^0 + b_1 * 2^1 + ... + b_750 * 2^750 + b_751 * 2^751
    /// We then calculate the hash like so:
    /// H = G_0 + s_1 * G_1 + ... + s_n * G_n
    /// where G_0 is a sum generator that is used to guarantee that the exponent of the resulting
    /// EC point is not known (necessary for BLS signatures).
    pub fn evaluate(
        input: &[Boolean<MNT6Fq>],
        generators: &[G1Var],
    ) -> Result<G1Var, SynthesisError> {
        let capacity = 752;

        // Check that the input can be stored using the available generators.
        assert!((generators.len() - 1) * capacity >= input.len());

        // Initiate the result with the sum generator.
        let mut result = generators[0].clone();

        // Start calculating the Pedersen hash. We use the double-and-add method for EC point
        // multiplication for each generator.
        // Note that Rust forces us to initialize the base to something.
        let mut base = G1Var::zero();
        for i in 0..input.len() {
            // Whenever i is a multiple of POINT_CAPACITY, it's time to get the next generator.
            if i % capacity == 0 {
                base = generators[i / capacity + 1].clone();
            }

            // Add the base to the result.
            let alt_result = &result + &base;

            // Depending on the bit, either select the new result (with the base added) or
            // continue with the previous result.
            result = G1Var::conditionally_select(&input[i], &alt_result, &result)?;

            // Double the base.
            base.double_in_place()?;
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt6_753::{constraints::G1Var, Fq as MNT6Fq};
    use ark_r1cs_std::{
        prelude::{AllocVar, Boolean},
        R1CSVar,
    };
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::test_rng;
    use rand::RngCore;

    use nimiq_bls::{
        pedersen::{pedersen_generator_powers, pedersen_generators, pedersen_hash},
        utils::bytes_to_bits_le,
    };
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
        let bits = bytes_to_bits_le(&bytes);

        // Generate the generators for the Pedersen hash.
        let generators = pedersen_generators(6);
        let generator_powers = pedersen_generator_powers(6);

        // Evaluate Pedersen hash using the primitive version.
        let primitive_hash = pedersen_hash(bits.clone(), &generator_powers);

        // Allocate the random bits in the circuit.
        let bits_var = Vec::<Boolean<MNT6Fq>>::new_witness(cs.clone(), || Ok(bits)).unwrap();

        // Allocate the Pedersen generators in the circuit.
        let generators_var = Vec::<G1Var>::new_witness(cs, || Ok(generators)).unwrap();

        // Evaluate Pedersen hash using the gadget version.
        let gadget_hash = PedersenHashGadget::evaluate(&bits_var, &generators_var).unwrap();

        assert_eq!(primitive_hash, gadget_hash.value().unwrap())
    }
}
