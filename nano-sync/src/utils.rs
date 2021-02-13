use ark_ec::ProjectiveCurve;
use ark_ff::{Field, FpParameters};
use ark_mnt4_753::{
    Fr as MNT4Fr, G1Projective as MNT4G1Projective, G2Projective as MNT4G2Projective,
};
use ark_mnt6_753::{
    Fr as MNT6Fr, G1Projective as MNT6G1Projective, G2Projective as MNT6G2Projective,
};
use ark_r1cs_std::boolean::Boolean;
use rand::{thread_rng, RngCore};

// Re-export bls utility functions.
pub use nimiq_bls::utils::*;

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

// TODO: Possibly remove this since it looks like we can generate EC points directly:
// let mut rng = test_rng();
// let a = MNT4Fr::rand(&mut rng);
/// Creates a random G1 point in the MNT4-753 curve.
pub fn gen_rand_g1_mnt4() -> MNT4G1Projective {
    let rng = &mut thread_rng();
    let mut bytes = [0u8; 96];
    let mut x = None;
    while x.is_none() {
        rng.fill_bytes(&mut bytes[2..]);
        x = MNT4Fr::from_random_bytes(&bytes);
    }
    let mut point = MNT4G1Projective::prime_subgroup_generator();
    point *= x.unwrap();
    point
}

/// Creates a random G2 point in the MNT4-753 curve.
pub fn gen_rand_g2_mnt4() -> MNT4G2Projective {
    let rng = &mut thread_rng();
    let mut bytes = [0u8; 96];
    let mut x = None;
    while x.is_none() {
        rng.fill_bytes(&mut bytes[2..]);
        x = MNT4Fr::from_random_bytes(&bytes);
    }
    let mut point = MNT4G2Projective::prime_subgroup_generator();
    point *= x.unwrap();
    point
}

/// Creates a random G1 point in the MNT6-753 curve.
pub fn gen_rand_g1_mnt6() -> MNT6G1Projective {
    let rng = &mut thread_rng();
    let mut bytes = [0u8; 96];
    let mut x = None;
    while x.is_none() {
        rng.fill_bytes(&mut bytes[2..]);
        x = MNT6Fr::from_random_bytes(&bytes);
    }
    let mut point = MNT6G1Projective::prime_subgroup_generator();
    point *= x.unwrap();
    point
}

/// Creates a random G2 point in the MNT6-753 curve.
pub fn gen_rand_g2_mnt6() -> MNT6G2Projective {
    let rng = &mut thread_rng();
    let mut bytes = [0u8; 96];
    let mut x = None;
    while x.is_none() {
        rng.fill_bytes(&mut bytes[2..]);
        x = MNT6Fr::from_random_bytes(&bytes);
    }
    let mut point = MNT6G2Projective::prime_subgroup_generator();
    point *= x.unwrap();
    point
}

/// Takes multiple bit representations of a point (Fp/Fp2/Fp3).
/// Its length must be a multiple of `P::MODULUS_BITS`.
/// None of the underlying points must be zero!
/// This function pads each chunk of `MODULUS_BITS` to full bytes, prepending the `y_bit`
/// in the very front.
/// This maintains *Big-Endian* representation.
// TODO: Can use just one field type parameter. Use F::size_in_bits() to get the modulus bits!
pub fn pad_point_bits<P: FpParameters, F: Field>(
    mut bits: Vec<Boolean<F>>,
    y_bit: Boolean<F>,
) -> Vec<Boolean<F>> {
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
