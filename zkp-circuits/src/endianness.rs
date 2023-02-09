use ark_ff::Field;
use ark_r1cs_std::{prelude::Boolean, uint32::UInt32, uint8::UInt8, ToBitsGadget, ToBytesGadget};
use ark_relations::r1cs::SynthesisError;

/// Takes a vector of bits and reverses the bit order within each byte.
/// The order of the bytes is left untouched.
fn reverse_inner_byte_order<F: Field>(data: &mut [Boolean<F>]) {
    assert_eq!(data.len() % 8, 0);

    // Reverse each 8 bit chunk.
    for chunk in data.chunks_mut(8) {
        chunk.reverse();
    }
}

/// The possible formats of the bits.
/// We do need to distinguish between bit and byte big endian because they are identical.
#[derive(Clone, Copy)]
pub enum Endianness {
    /// The bits are ordered from least-significant to most-significant bit.
    /// This is the common representation of bits in arkworks.
    BitLittleEndian,
    /// The bytes are ordered from least-significant to most-significant,
    /// but the bit order within each byte is from most-significant bit to least-significant bit.
    /// Corresponds to what is commonly referred to as Little Endian.
    ByteLittleEndian,
    /// The bits are ordered from most-significant to least-significant bit.
    /// Corresponds to what is commonly referred to as Big Endian.
    BigEndian,
}

/// A wrapper for the bits and its representation.
#[derive(Clone)]
pub struct Bits<F: Field> {
    /// The raw bits ordered according to the endianness.
    bits: Vec<Boolean<F>>,
    /// The endianness format of the bits.
    endianness: Endianness,
}

impl<F: Field> Bits<F> {
    pub fn new(bits: Vec<Boolean<F>>, endianness: Endianness) -> Self {
        Self { bits, endianness }
    }

    /// Reorder the bits as bit little endian.
    pub fn bit_le(&mut self) {
        match self.endianness {
            Endianness::BitLittleEndian => {}
            Endianness::ByteLittleEndian => reverse_inner_byte_order(&mut self.bits),
            Endianness::BigEndian => self.bits.reverse(),
        };
        self.endianness = Endianness::BitLittleEndian;
    }

    /// Reorder the bits as byte little endian.
    pub fn byte_le(&mut self) {
        match self.endianness {
            Endianness::BitLittleEndian => reverse_inner_byte_order(&mut self.bits),
            Endianness::ByteLittleEndian => {}
            Endianness::BigEndian => {
                self.bits.reverse();
                reverse_inner_byte_order(&mut self.bits);
            }
        };
        self.endianness = Endianness::ByteLittleEndian;
    }

    /// Reorder the bits as big endian.
    pub fn be(&mut self) {
        match self.endianness {
            Endianness::BitLittleEndian => self.bits.reverse(),
            Endianness::ByteLittleEndian => {
                reverse_inner_byte_order(&mut self.bits);
                self.bits.reverse();
            }
            Endianness::BigEndian => {}
        };
        self.endianness = Endianness::ByteLittleEndian;
    }

    /// Returns the bits as bit little endian.
    pub fn as_bit_le_bits(&self) -> Vec<Boolean<F>> {
        let mut bits = self.clone();
        bits.bit_le();
        bits.bits
    }

    /// Returns the bits as byte little endian.
    pub fn as_byte_le_bits(&self) -> Vec<Boolean<F>> {
        let mut bits = self.clone();
        bits.byte_le();
        bits.bits
    }

    /// Returns the bits as big endian.
    pub fn as_be_bits(&self) -> Vec<Boolean<F>> {
        let mut bits = self.clone();
        bits.be();
        bits.bits
    }

    /// Return the format of the bits.
    pub fn endianness(&self) -> Endianness {
        self.endianness
    }

    /// Returns the bits on its current endianness.
    pub fn raw_bits(&self) -> &[Boolean<F>] {
        &self.bits
    }
}

impl<F: Field> From<Vec<UInt8<F>>> for Bits<F> {
    fn from(value: Vec<UInt8<F>>) -> Self {
        Self::new(
            value
                .into_iter()
                .flat_map(|byte| byte.to_bits_le().unwrap())
                .collect(),
            Endianness::BitLittleEndian,
        )
    }
}

impl<F: Field> From<Vec<UInt32<F>>> for Bits<F> {
    fn from(value: Vec<UInt32<F>>) -> Self {
        Self::new(
            value.into_iter().flat_map(|int| int.to_bits_le()).collect(),
            Endianness::BitLittleEndian,
        )
    }
}

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

#[cfg(test)]
mod tests {
    use ark_test_curves::bls12_381::Fr;
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn bit_format_conversion_works() {
        // 0x011011
        const BE_BITS: [Boolean<Fr>; 8 * 3] = [
            // 1st byte: 0x01
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::TRUE,
            // 2nd byte: 0x10
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::TRUE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            // 3rd byte: 0x11
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::TRUE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::TRUE,
        ];

        const BIT_LE_BITS: [Boolean<Fr>; 8 * 3] = [
            // 1st byte
            Boolean::TRUE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::TRUE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            // 2nd byte
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::TRUE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            // 3rd byte
            Boolean::TRUE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
        ];

        const BYTE_LE_BITS: [Boolean<Fr>; 8 * 3] = [
            // 1st byte
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::TRUE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::TRUE,
            // 2nd byte
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::TRUE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            // 3rd byte
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::FALSE,
            Boolean::TRUE,
        ];

        // Start from be
        let bits = Bits::new(BE_BITS.to_vec(), Endianness::BigEndian);
        assert_eq!(&bits.as_be_bits(), &BE_BITS);
        assert_eq!(&bits.as_bit_le_bits(), &BIT_LE_BITS);
        assert_eq!(&bits.as_byte_le_bits(), &BYTE_LE_BITS);

        // Start from bit le
        let bits = Bits::new(BIT_LE_BITS.to_vec(), Endianness::BitLittleEndian);
        assert_eq!(&bits.as_be_bits(), &BE_BITS);
        assert_eq!(&bits.as_bit_le_bits(), &BIT_LE_BITS);
        assert_eq!(&bits.as_byte_le_bits(), &BYTE_LE_BITS);

        // Start from byte le
        let bits = Bits::new(BYTE_LE_BITS.to_vec(), Endianness::ByteLittleEndian);
        assert_eq!(&bits.as_be_bits(), &BE_BITS);
        assert_eq!(&bits.as_bit_le_bits(), &BIT_LE_BITS);
        assert_eq!(&bits.as_byte_le_bits(), &BYTE_LE_BITS);
    }
}
