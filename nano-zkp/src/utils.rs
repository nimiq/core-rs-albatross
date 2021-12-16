use std::cmp::min;

use ark_ec::ProjectiveCurve;
use ark_ff::{Field, PrimeField};
use ark_mnt6_753::{Fr as MNT6Fr, G2Projective as G2MNT6};
use ark_r1cs_std::fields::fp::FpVar;
use ark_r1cs_std::prelude::{Boolean, ToBitsGadget};
use ark_relations::r1cs::SynthesisError;
use ark_std::ops::MulAssign;
use ark_std::UniformRand;
use rand::prelude::SliceRandom;
use rand::rngs::SmallRng;
use rand::{RngCore, SeedableRng};

use nimiq_nano_primitives::{pk_tree_construct, state_commitment, MacroBlock};
use nimiq_primitives::policy::{EPOCH_LENGTH, SLOTS, TWO_F_PLUS_ONE};

/// Takes a vector of booleans and converts it into a vector of field elements, which is the way we
/// represent inputs to circuits (natively).
/// It assumes the bits are in little endian.
/// This function is meant to be used off-circuit.
pub fn pack_inputs<F: PrimeField>(mut input: Vec<bool>) -> Vec<F> {
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
/// transformed into field elements, which is the way we represent inputs to circuits (as a gadget).
/// This assumes that both the constraint field and the target field have the same size in bits
/// (which is true for the MNT curves).
/// Each field element has his last bit set to zero (since the capacity of a field is always one bit
/// less than its size). We also pad the last field element with zeros so that it has the correct
/// size.
/// This function is meant to be used on-circuit.
pub fn prepare_inputs<F: PrimeField>(mut input: Vec<Boolean<F>>) -> Vec<Vec<Boolean<F>>> {
    let capacity = F::size_in_bits() - 1;

    let mut result = vec![];

    while !input.is_empty() {
        let length = min(input.len(), capacity);

        let padding = F::size_in_bits() - length;

        let remaining_input = input.split_off(length);

        for _ in 0..padding {
            input.push(Boolean::constant(false));
        }

        result.push(input);

        input = remaining_input;
    }

    result
}

/// Takes a vector of public inputs to a circuit, represented as field elements, and converts it
/// to the canonical representation of a vector of Booleans. Internally, it just converts the field
/// elements to bits and discards the most significant bit (which never contains any data).
/// This function is meant to be used on-circuit.
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

    // The serialization begins with the y_bit, followed by the infinity flag.
    let mut serialization = vec![y_bit, infinity_bit];

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

/// Create a macro block, validator keys and other information needed to produce a nano-zkp SNARK
/// proof. It is used in the examples. It takes as input an index that represents the epoch that we are in.
/// Note that the RNG and seed aren't secure enough, so this function should only be used for test purposes.
pub fn create_test_blocks(
    index: u64,
) -> (
    Vec<G2MNT6>,
    [u8; 32],
    Vec<G2MNT6>,
    MacroBlock,
    Option<Vec<u8>>,
) {
    // The random seed. It was generated using random.org.
    let seed = 12370426996209291122;

    // Create RNG.
    let mut rng = SmallRng::seed_from_u64(seed + index);

    // Create key pairs for the initial validators.
    let mut initial_sks = vec![];
    let mut initial_pks = vec![];

    for _ in 0..SLOTS {
        let sk = MNT6Fr::rand(&mut rng);
        let mut pk = G2MNT6::prime_subgroup_generator();
        pk.mul_assign(sk);
        initial_sks.push(sk);
        initial_pks.push(pk);
    }

    // Create the initial header hash.
    let mut initial_header_hash = [0u8; 32];
    rng.fill_bytes(&mut initial_header_hash);

    // Create a random signer bitmap.
    let mut signer_bitmap = vec![true; TWO_F_PLUS_ONE as usize];

    signer_bitmap.append(&mut vec![false; (SLOTS - TWO_F_PLUS_ONE) as usize]);

    signer_bitmap.shuffle(&mut rng);

    // Restart the RNG with the next index.
    let mut rng = SmallRng::seed_from_u64(seed + index + 1);

    // Create key pairs for the final validators.
    let mut final_sks = vec![];
    let mut final_pks = vec![];

    for _ in 0..SLOTS {
        let sk = MNT6Fr::rand(&mut rng);
        let mut pk = G2MNT6::prime_subgroup_generator();
        pk.mul_assign(sk);
        final_sks.push(sk);
        final_pks.push(pk);
    }

    // Create the final header hash.
    let mut final_header_hash = [0u8; 32];
    rng.fill_bytes(&mut final_header_hash);

    // There is no more randomness being generated from this point on.

    // Calculate final public key tree root.
    let final_pk_tree_root = pk_tree_construct(final_pks.clone());

    // Create the macro block.
    let mut block =
        MacroBlock::without_signatures(EPOCH_LENGTH * (index as u32 + 1), 0, final_header_hash);

    for i in 0..SLOTS as usize {
        if signer_bitmap[i] {
            block.sign(&initial_sks[i], i, &final_pk_tree_root);
        }
    }

    // If this is the first index (genesis), also return the genesis state commitment.
    let genesis_state_commitment = if index == 0 {
        Some(state_commitment(
            0,
            initial_header_hash,
            initial_pks.clone(),
        ))
    } else {
        None
    };

    // Return the data.
    (
        initial_pks,
        initial_header_hash,
        final_pks,
        block,
        genesis_state_commitment,
    )
}
