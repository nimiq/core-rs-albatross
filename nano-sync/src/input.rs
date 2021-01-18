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
        let mut point = F::zero();
        let mut base = F::one();

        let new_input = input.split_off(capacity);

        for bit in input {
            if bit {
                point += base;
            }
            base.double_in_place();
        }

        result.push(point);

        input = new_input;
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
