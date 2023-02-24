use ark_ff::Field;
use ark_r1cs_std::{uint32::UInt32, uint8::UInt8, ToBytesGadget};
use ark_relations::r1cs::SynthesisError;

/// Specifies constraints for conversion to a big-endian byte representation
/// of `self`.
pub trait ToBeBytesGadget<F: Field> {
    /// Outputs a canonical, big-endian, byte decomposition of `self`.
    fn to_bytes_be(&self) -> Result<Vec<UInt8<F>>, SynthesisError>;
}

impl<F: Field> ToBeBytesGadget<F> for UInt32<F> {
    fn to_bytes_be(&self) -> Result<Vec<UInt8<F>>, SynthesisError> {
        // Get the big-endian byte representation by reversing the little-endian byte representation.
        // We do not care about the order of the bits within each byte.
        let mut bytes = self.to_bytes()?;
        bytes.reverse();
        Ok(bytes)
    }
}
