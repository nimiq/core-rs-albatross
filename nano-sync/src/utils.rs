use std::cmp::min;

use ark_ff::{Field, PrimeField};
use ark_r1cs_std::fields::fp::FpVar;
use ark_r1cs_std::prelude::{Boolean, ToBitsGadget};
use ark_relations::r1cs::SynthesisError;

// Re-export bls utility functions.
pub use nimiq_bls::utils::*;

/// Takes a vector of booleans and converts it into a vector of field elements, which is the way we
/// represent inputs to circuits.
pub fn prepare_inputs<F: PrimeField>(mut input: Vec<bool>) -> Vec<F> {
    let capacity = F::size_in_bits() - 1;

    let mut result = vec![];

    while !input.is_empty() {
        let length = min(input.len(), capacity);

        let remaining_input = input.split_off(length);

        let mut point = F::zero();

        let mut base = F::one();

        for bit in input {
            if bit {
                point += base;
            }
            base.double_in_place();
        }

        result.push(point);

        input = remaining_input;
    }

    result
}

/// Takes a vector of Booleans and transforms it into a vector of a vector of Booleans, ready to be
/// transformed into field elements, which is the way we represent inputs to circuits. This assumes
/// that both the constraint field and the target field have the same size in bits (which is true
/// for the MNT curves).
/// Each field element has his last bit set to zero (since the capacity of a field is always one bit
/// less than its size). We also pad the last field element with zeros so that it has the correct
/// size.
pub fn pack_inputs<F: PrimeField>(mut input: Vec<Boolean<F>>) -> Vec<Vec<Boolean<F>>> {
    let capacity = F::size_in_bits() - 1;

    let mut result = vec![];

    while !input.is_empty() {
        let length = min(input.len(), capacity);

        let padding = F::size_in_bits() - length;

        let new_input = input.split_off(length);

        for _ in 0..padding {
            input.push(Boolean::constant(false));
        }

        result.push(input);

        input = new_input;
    }

    result
}

/// Takes a vector of public inputs to a circuit, represented as field elements, and converts it
/// to the canonical representation of a vector of Booleans. Internally, it just converts the field
/// elements to bits and discards the most significant bit (which never contains any data).
pub fn unpack_inputs<F: PrimeField>(
    inputs: Vec<FpVar<F>>,
) -> Result<Vec<Boolean<F>>, SynthesisError> {
    let mut result = vec![];

    for elem in inputs {
        let mut bits = elem.to_bits_le()?;
        bits.pop();
        result.append(&mut bits);
    }

    Ok(result)
}

/// Takes the bit representation of a point coordinate (like Fp, Fp2,
/// Fp3, etc) and pads each field element to full bytes, prepending y_bit and infinity_bit in the
/// very front of the serialization.
/// The input length must be a multiple of the field size (in bits).
/// This assumes the field elements come in little-endian, but it outputs in big-endian.
pub fn pad_point_bits<F: PrimeField>(
    mut bits: Vec<Boolean<F>>,
    y_bit: Boolean<F>,
    infinity_bit: Boolean<F>,
) -> Vec<Boolean<F>> {
    let point_len = F::size_in_bits();

    assert_eq!(bits.len() % point_len, 0,);

    let padding = 8 - (point_len % 8);

    let mut serialization = vec![];

    // The serialization begins with the y_bit, followed by the infinity flag.
    serialization.push(y_bit);
    serialization.push(infinity_bit);

    for i in 0..bits.len() / point_len {
        // If we are in the first round, skip two bits of padding.
        let padding_len = if i == 0 { padding - 2 } else { padding };

        // Add the padding.
        for _ in 0..padding_len {
            serialization.push(Boolean::constant(false));
        }

        // Split bits at point_len. Now new_bits contains the elements in the range [MODULUS, len).
        let new_bits = bits.split_off(point_len as usize);

        // Reverse the bits to get the big-endian representation.
        bits.reverse();

        // Append the bits to the serialization.
        serialization.append(&mut bits);

        bits = new_bits;
    }

    serialization
}

/// Takes a data vector in *Big-Endian* representation and transforms it,
/// such that each byte starts with the least significant bit (as expected by blake2 gadgets).
/// b0 b1 b2 b3 b4 b5 b6 b7 b8 -> b8 b7 b6 b5 b4 b3 b2 b1 b0
pub fn reverse_inner_byte_order<F: Field>(data: &[Boolean<F>]) -> Vec<Boolean<F>> {
    assert_eq!(data.len() % 8, 0);

    data.chunks(8)
        // Reverse each 8 bit chunk.
        .flat_map(|chunk| chunk.iter().rev().cloned())
        .collect::<Vec<Boolean<F>>>()
}

/// Transforms a vector of little endian bits into a u8.
pub fn byte_from_le_bits(bits: &[bool]) -> u8 {
    assert!(bits.len() <= 8);

    let mut byte = 0;
    let mut base = 1;

    for i in 0..bits.len() {
        if bits[i] {
            byte += base;
        }
        base *= 2;
    }

    byte
}

/// Transforms a u8 into a vector of little endian bits.
pub fn byte_to_le_bits(mut byte: u8) -> Vec<bool> {
    let mut bits = vec![];

    for _ in 0..8 {
        bits.push(byte % 2 != 0);
        byte = byte >> 1;
    }

    bits
}
