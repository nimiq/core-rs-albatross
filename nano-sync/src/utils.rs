use std::cmp::min;

use ark_ff::{Field, PrimeField};
use ark_r1cs_std::fields::fp::FpVar;
use ark_r1cs_std::prelude::{Boolean, ToBitsGadget};
use ark_relations::r1cs::SynthesisError;

// Re-export bls utility functions.
pub use nimiq_bls::utils::*;

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
