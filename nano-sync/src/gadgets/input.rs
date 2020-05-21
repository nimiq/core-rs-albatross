use algebra_core::biginteger::BigInteger;
use algebra_core::{FpParameters, PrimeField};
use r1cs_core::SynthesisError;
use r1cs_std::uint8::UInt8;

/// This is a gadget that packs a `Vec<Uint8>` into field elements of a target field
/// `TargetConstraintF`, so that the vector can be read using `Uint8::alloc_input_vec`
/// on a circuit with this target field.
pub struct RecursiveInputGadget;

impl RecursiveInputGadget {
    /// Converts a `Vec<Uint8>` into field elements, each represented by a `Vec<UInt8>`,
    /// that can be read by `Uint8::alloc_input_vec` when passing to a proof verification.
    pub fn to_field_elements<TargetConstraintF: PrimeField>(
        input_bytes: &[UInt8],
    ) -> Result<Vec<Vec<UInt8>>, SynthesisError> {
        let max_size = TargetConstraintF::Params::CAPACITY / 8;

        let max_size = max_size as usize;

        let bigint_size = TargetConstraintF::BigInt::NUM_LIMBS * 8;

        let fes = input_bytes
            .chunks(max_size)
            .map(|chunk| {
                let mut chunk = chunk.to_vec();
                let len = chunk.len();
                for _ in len..bigint_size {
                    chunk.push(UInt8::constant(0));
                }
                chunk
            })
            .collect::<Vec<_>>();

        Ok(fes)
    }
}
