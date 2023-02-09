use ark_crypto_primitives::prf::blake2s::constraints;
use ark_ff::PrimeField;
use ark_r1cs_std::{uint8::UInt8, ToBitsGadget, ToBytesGadget};
use ark_relations::r1cs::SynthesisError;

pub fn evaluate_blake2s<ConstraintF: PrimeField>(
    input: &[UInt8<ConstraintF>],
) -> Result<Vec<UInt8<ConstraintF>>, SynthesisError> {
    let hash = constraints::evaluate_blake2s(&input.to_bits_le()?)?;
    Ok(hash
        .into_iter()
        .flat_map(|int| int.to_bytes().unwrap())
        .collect())
}

pub fn evaluate_blake2s_with_parameters<F: PrimeField>(
    input: &[UInt8<F>],
    parameters: &[u32; 8],
) -> Result<Vec<UInt8<F>>, SynthesisError> {
    let hash = constraints::evaluate_blake2s_with_parameters(&input.to_bits_le()?, parameters)?;
    Ok(hash
        .into_iter()
        .flat_map(|int| int.to_bytes().unwrap())
        .collect())
}
