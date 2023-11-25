use std::{cmp::min, marker::PhantomData};

use ark_crypto_primitives::snark::BooleanInputVar;
use ark_ff::PrimeField;
use ark_r1cs_std::{prelude::Boolean, ToBitsGadget};
use ark_relations::r1cs::SynthesisError;

#[derive(Debug, Clone)]
pub struct RecursiveInputVar<F: PrimeField, CF: PrimeField> {
    pub(crate) val: Vec<Vec<Boolean<CF>>>,
    _snark_field_: PhantomData<F>,
}

impl<F: PrimeField, CF: PrimeField> Default for RecursiveInputVar<F, CF> {
    fn default() -> Self {
        RecursiveInputVar {
            val: vec![],
            _snark_field_: PhantomData,
        }
    }
}
impl<F: PrimeField, CF: PrimeField> RecursiveInputVar<F, CF> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<T: ToBitsGadget<CF> + ?Sized>(&mut self, item: &T) -> Result<(), SynthesisError> {
        let bits = item.to_bits_le()?;
        self.val.append(&mut Self::repack_input(bits));

        Ok(())
    }

    pub fn append(&mut self, boolean_input: BooleanInputVar<F, CF>) -> Result<(), SynthesisError> {
        self.val.extend(boolean_input);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.val.len()
    }

    pub fn is_empty(&self) -> bool {
        self.val.is_empty()
    }

    /// Takes a vector of Booleans and transforms it into a vector of a vector of Booleans, ready to be
    /// transformed into field elements, which is the way we represent inputs to circuits (as a gadget).
    /// This assumes that both the constraint field and the target field have the same size in bits
    /// (which is true for the MNT curves).
    /// Each field element has his last bit set to zero (since the capacity of a field is always one bit
    /// less than its size). We also pad the last field element with zeros so that it has the correct
    /// size.
    /// This function is meant to be used on-circuit.
    fn repack_input(mut input: Vec<Boolean<CF>>) -> Vec<Vec<Boolean<CF>>> {
        let capacity = F::MODULUS_BIT_SIZE as usize - 1;

        let mut result = vec![];

        while !input.is_empty() {
            let length = min(input.len(), capacity);
            let padding = F::MODULUS_BIT_SIZE as usize - length;
            let remaining_input = input.split_off(length);

            for _ in 0..padding {
                input.push(Boolean::FALSE);
            }

            result.push(input);
            input = remaining_input;
        }

        result
    }
}

impl<F: PrimeField, CF: PrimeField> From<RecursiveInputVar<F, CF>> for BooleanInputVar<F, CF> {
    fn from(value: RecursiveInputVar<F, CF>) -> Self {
        BooleanInputVar::new(value.val)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt4_753::Fq as MNT4Fq;
    use ark_mnt6_753::Fq as MNT6Fq;
    use ark_r1cs_std::{uint8::UInt8, R1CSVar};

    use super::*;

    const BYTES: [u8; 95] = [
        227, 90, 6, 29, 55, 139, 106, 148, 42, 203, 6, 18, 181, 134, 13, 109, 178, 112, 145, 3,
        118, 242, 146, 181, 148, 208, 175, 42, 201, 78, 106, 63, 49, 34, 115, 234, 249, 127, 92,
        37, 237, 74, 204, 75, 215, 134, 154, 6, 194, 223, 211, 186, 143, 10, 50, 173, 241, 45, 127,
        147, 200, 254, 178, 106, 149, 76, 135, 65, 212, 7, 168, 171, 6, 186, 191, 37, 178, 200,
        154, 211, 181, 89, 25, 52, 248, 17, 65, 106, 33, 17, 177, 123, 119, 65, 129,
    ];

    fn bits_to_byte(bits: &[bool]) -> u8 {
        // assert_eq!(bits.len(), 8);
        let mut n = 0;
        for (i, b) in bits.iter().enumerate() {
            if *b {
                n += 1 << i;
            }
        }
        n
    }

    fn test_conversion<FFrom: PrimeField, FTo: PrimeField>() {
        // Convert bytes to bits on other field.
        let input = UInt8::<FFrom>::constant_vec(&BYTES);
        let mut r_input = RecursiveInputVar::<FTo, FFrom>::new();
        r_input.push(&input).unwrap();

        // Convert back to bytes on target field.
        let max_size = 8 * ((FTo::MODULUS_BIT_SIZE - 1) / 8) as usize;
        let mut allocated_bytes = Vec::new();

        for fe_bits in r_input.val.into_iter() {
            // Remove the most significant bit, because we know it should be zero
            // because `values.to_field_elements()` only
            // packs field elements up to the penultimate bit.
            // That is, the most significant bit (`ConstraintF::NUM_BITS`-th bit) is
            // unset, so we can just pop it off.
            let fe_bytes: Vec<_> = fe_bits[0..max_size]
                .chunks(8)
                .map(|b| bits_to_byte(&b.value().unwrap()))
                .collect();
            allocated_bytes.extend_from_slice(&fe_bytes);
        }

        let mut padded_bytes = BYTES.to_vec();
        padded_bytes.resize(allocated_bytes.len(), 0);

        assert_eq!(allocated_bytes, padded_bytes);
    }

    #[test]
    fn test_conversion_mnt6_mnt4() {
        test_conversion::<MNT6Fq, MNT4Fq>()
    }

    #[test]
    fn test_conversion_mnt4_mnt6() {
        test_conversion::<MNT4Fq, MNT6Fq>()
    }
}
