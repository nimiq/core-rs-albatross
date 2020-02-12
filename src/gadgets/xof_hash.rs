use algebra::fields::{sw6::Fr as SW6Fr, sw6::FrParameters as SW6FrParameters, FpParameters};
use crypto_primitives::prf::blake2s::constraints::blake2s_gadget_with_parameters;
use crypto_primitives::prf::Blake2sWithParameterBlock;
use r1cs_core::SynthesisError;
use r1cs_std::bits::boolean::Boolean;

pub type HashToBitsField = SW6Fr; // TODO: Why does celo use Bls12_377Fr? To create a NIZK?
pub type HashToBitsFieldParameters = SW6FrParameters;

/// See https://gist.github.com/SooryaN/8d1b2c19bf0b971c11366b0680908d4b
pub struct XofHashGadget {}

impl XofHashGadget {
    /// Since we need 377 bit for our try and increment method, the hash output has to yield at least 48 bytes.
    pub const XOF_DIGEST_LENGTH: u16 =
        ((HashToBitsFieldParameters::MODULUS_BITS + 1 + 8 - 1) / 8) as u16; // ceil(378/8) = 48

    pub fn xof_hash<CS: r1cs_core::ConstraintSystem<HashToBitsField>>(
        mut cs: CS,
        hash: &[Boolean],
    ) -> Result<Vec<Boolean>, SynthesisError> {
        // Number of iterations is ceil(XOF_DIGEST_LENGTH / 32).
        let iterations = ((Self::XOF_DIGEST_LENGTH + 32 - 1) / 32) as usize;
        let mut xof_bits = vec![];
        for i in 0..iterations {
            // Digest length is 32 for all except the last iteration.
            // If XOF_DIGEST_LENGTH % 32 != 0, the digest_length for the last iteration
            // needs to be set to this value.
            let digest_length = if i + 1 == iterations && Self::XOF_DIGEST_LENGTH % 32 != 0 {
                (Self::XOF_DIGEST_LENGTH % 32) as u8
            } else {
                32
            };
            let blake2s_parameters = Blake2sWithParameterBlock {
                digest_length,                              // ok
                key_length: 0,                              // default parameters
                fan_out: 0,                                 // ok
                depth: 0,                                   // ok
                leaf_length: 32,                            // ok
                node_offset: i as u32,                      // ok
                xof_digest_length: Self::XOF_DIGEST_LENGTH, // ok
                node_depth: 0,                              // ok
                inner_length: 32,                           // ok
                salt: [0; 8],                               // default parameters
                personalization: [0; 8],                    // default parameters
            };
            let xof_result = blake2s_gadget_with_parameters(
                cs.ns(|| format!("xof result {}", i)),
                &hash,
                &blake2s_parameters.parameters(),
            )?;
            let xof_bits_i = xof_result
                .into_iter()
                .map(|n| n.to_bits_le())
                .flatten()
                .collect::<Vec<Boolean>>();
            xof_bits.extend_from_slice(&xof_bits_i);
        }

        // Forget unnecessary bits.
        xof_bits.truncate(HashToBitsFieldParameters::MODULUS_BITS as usize + 1);
        Ok(xof_bits)
    }
}
