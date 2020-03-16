use super::*;

/// Calculates the Blake2s hash for the block from:
/// block number || public_keys.
pub fn calculate_state_hash<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
    mut cs: CS,
    block_number: &UInt32,
    public_keys: &Vec<G2Gadget<Bls12_377Parameters>>,
) -> Result<Vec<UInt32>, SynthesisError> {
    // Initialize Boolean vector.
    let mut bits: Vec<Boolean> = vec![];

    // The block number comes in little endian all the way.
    // So, a reverse will put it into big endian.
    let mut block_number_be = block_number.to_bits_le();
    block_number_be.reverse();
    bits.append(&mut block_number_be);

    // Convert each public key to bits and append it.
    for key in public_keys.iter() {
        // Get bits from the x coordinate.
        let x_bits: Vec<Boolean> = key.x.to_bits(cs.ns(|| "pks to bits"))?;
        // Get one bit from the y coordinate.
        let greatest_bit =
            YToBitGadget::<Bls12_377Parameters>::y_to_bit_g2(cs.ns(|| "y to bit"), key)?;
        // Pad points and get *Big-Endian* representation.
        let mut serialized_bits = pad_point_bits::<FqParameters>(x_bits, greatest_bit);
        // Append to Boolean vector.
        bits.append(&mut serialized_bits);
    }

    // TODO: Is this needed?
    // Prepare order of booleans for blake2s (it doesn't expect Big-Endian).
    let bits = reverse_inner_byte_order(&bits);
    let input_bytes: Vec<UInt8> = bits
        .chunks(8)
        .map(|chunk| UInt8::from_bits_le(chunk))
        .collect();

    blake2s_gadget(cs.ns(|| "blake2s hash from serialized bits"), &bits)
}
