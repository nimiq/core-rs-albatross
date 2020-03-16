use algebra::FpParameters;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::bits::uint32::UInt32;

pub mod alloc_constant;
pub mod check_sig;
pub mod crh;
pub mod macro_block;
pub mod smaller_than;
pub mod state_hash;
pub mod y_to_bit;

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

/// Takes multiple bit representations of a point (Fp/Fp2).
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

/// Takes a hash output and returns the *Big-Endian* representation of it.
pub fn hash_to_bits(hash: Vec<UInt32>) -> Vec<Boolean> {
    hash.into_iter()
        .flat_map(|n| reverse_inner_byte_order(&n.to_bits_le()))
        .collect::<Vec<Boolean>>()
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
