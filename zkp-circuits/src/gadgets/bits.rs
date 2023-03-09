use ark_ff::{Field, PrimeField};
use ark_r1cs_std::{
    prelude::{AllocVar, Boolean},
    uint8::UInt8,
    ToBitsGadget,
};
use ark_relations::r1cs::{Namespace, SynthesisError};

pub struct BitVec<F: Field>(pub Vec<Boolean<F>>);

impl<F: Field> BitVec<F> {
    pub(crate) fn new_witness_vec(
        cs: impl Into<Namespace<F>>,
        values: &[impl Into<Option<bool>> + Copy],
    ) -> Result<Self, SynthesisError> {
        let ns = cs.into();
        let cs = ns.cs();

        let mut output_vec = Vec::with_capacity(values.len());
        for value in values {
            let bit: Option<bool> = Into::into(*value);
            output_vec.push(Boolean::<F>::new_witness(cs.clone(), || {
                bit.ok_or(SynthesisError::AssignmentMissing)
            })?);
        }
        Ok(BitVec(output_vec))
    }

    pub(crate) fn new_input_vec(
        cs: impl Into<Namespace<F>>,
        values: &[bool],
    ) -> Result<Self, SynthesisError>
    where
        F: PrimeField,
    {
        let values = Self::to_bytes_le(values);
        let bytes = UInt8::<F>::new_input_vec(cs, &values)?;
        Ok(BitVec(bytes.to_bits_le()?))
    }

    pub fn to_bytes_le(values: &[bool]) -> Vec<u8> {
        values
            .chunks(8)
            .map(|chunk| {
                let mut byte = 0;
                for (i, set) in chunk.iter().enumerate() {
                    if *set {
                        byte |= 1 << i;
                    }
                }
                byte
            })
            .collect()
    }
}
