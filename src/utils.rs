use algebra::mnt4_753::{G1Projective as MNT4G1Projective, G2Projective as MNT4G2Projective};
use algebra::mnt6_753::{G1Projective as MNT6G1Projective, G2Projective as MNT6G2Projective};
use algebra::{BigInteger768, FpParameters};
use algebra_core::ProjectiveCurve;
use r1cs_std::prelude::*;

use crate::compression::BeSerialize;

/// Serializes a G1 point in the MNT4-753 curve.
pub fn serialize_g1_mnt4(point: MNT4G1Projective) -> [u8; 95] {
    let mut buffer = [0u8; 95];
    BeSerialize::serialize(&point.into_affine(), &mut &mut buffer[..]).unwrap();
    buffer
}

/// Serializes a G2 point in the MNT4-753 curve.
pub fn serialize_g2_mnt4(point: MNT4G2Projective) -> [u8; 190] {
    let mut buffer = [0u8; 190];
    BeSerialize::serialize(&point.into_affine(), &mut &mut buffer[..]).unwrap();
    buffer
}

/// Serializes a G1 point in the MNT6-753 curve.
pub fn serialize_g1_mnt6(point: MNT6G1Projective) -> [u8; 95] {
    let mut buffer = [0u8; 95];
    BeSerialize::serialize(&point.into_affine(), &mut &mut buffer[..]).unwrap();
    buffer
}

/// Serializes a G2 point in the MNT6-753 curve.
pub fn serialize_g2_mnt6(point: MNT6G2Projective) -> [u8; 285] {
    let mut buffer = [0u8; 285];
    BeSerialize::serialize(&point.into_affine(), &mut &mut buffer[..]).unwrap();
    buffer
}

/// Takes multiple bit representations of a point (Fp/Fp2/Fp3).
/// Its length must be a multiple of `P::MODULUS_BITS`.
/// None of the underlying points must be zero!
/// This function pads each chunk of `MODULUS_BITS` to full bytes, prepending the `y_bit`
/// in the very front.
/// This maintains *Big-Endian* representation.
pub fn pad_point_bits<P: FpParameters>(mut bits: Vec<Boolean>, y_bit: Boolean) -> Vec<Boolean> {
    let point_len = P::MODULUS_BITS;

    let padding = 8 - (point_len % 8);

    assert_eq!(
        bits.len() % point_len as usize,
        0,
        "Can only pad multiples of point size"
    );

    let mut serialization = vec![];

    // Start with y_bit.
    serialization.push(y_bit);

    let mut first = true;

    while !bits.is_empty() {
        // First, add padding.
        // If we are in the first round, skip one bit of padding.
        // The serialization begins with the y_bit, followed by the infinity flag.
        // By definition, the point must not be infinity, thus we can skip this flag.
        let padding_len = if first {
            first = false;
            padding - 1
        } else {
            padding
        };

        for _ in 0..padding_len {
            serialization.push(Boolean::constant(false));
        }

        // Then, split bits at `MODULUS_BITS`:
        // `new_bits` contains the elements in the range [MODULUS, len).
        let new_bits = bits.split_off(point_len as usize);

        serialization.append(&mut bits);

        bits = new_bits;
    }

    assert_eq!(
        serialization.len() % 8,
        0,
        "Padded serialization should be of byte length"
    );

    serialization
}

/// Takes a data vector in *Big-Endian* representation and transforms it,
/// such that each byte starts with the least significant bit (as expected by blake2 gadgets).
/// b0 b1 b2 b3 b4 b5 b6 b7 b8 -> b8 b7 b6 b5 b4 b3 b2 b1 b0
pub fn reverse_inner_byte_order(data: &[Boolean]) -> Vec<Boolean> {
    assert_eq!(data.len() % 8, 0);

    data.chunks(8)
        // Reverse each 8 bit chunk.
        .flat_map(|chunk| chunk.iter().rev().cloned())
        .collect::<Vec<Boolean>>()
}

/// Transforms a vector of bytes into the corresponding vector of bits (booleans).
pub fn bytes_to_bits(bytes: &[u8]) -> Vec<bool> {
    let mut bits = vec![];

    for i in 0..bytes.len() {
        let byte = bytes[i];
        for j in (0..8).rev() {
            bits.push((byte >> j) & 1 == 1);
        }
    }

    bits
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

/// Cretes a BigInteger from an array of bytes in big-endian format.
pub fn big_int_from_bytes_be<R: std::io::Read>(reader: &mut R) -> BigInteger768 {
    let mut res = [0u64; 12];

    for num in res.iter_mut().rev() {
        let mut bytes = [0u8; 8];
        reader.read_exact(&mut bytes).unwrap();
        *num = u64::from_be_bytes(bytes);
    }

    BigInteger768::new(res)
}
