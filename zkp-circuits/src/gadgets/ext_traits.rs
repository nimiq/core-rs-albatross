use ark_crypto_primitives::snark::BooleanInputVar;
use ark_ff::PrimeField;
use ark_r1cs_std::{boolean::Boolean, uint8::UInt8};

pub trait ToUncompressedBytesGadget<F: PrimeField> {
    fn to_bytes(&self) -> Vec<UInt8<F>>;
}

impl<F: PrimeField, CF: PrimeField> ToUncompressedBytesGadget<CF> for BooleanInputVar<F, CF> {
    fn to_bytes(&self) -> Vec<UInt8<CF>> {
        let bits = self
            .clone()
            .into_iter()
            .flat_map(|mut bits| {
                bits.resize_with(bits.len().div_ceil(8) * 8, || Boolean::FALSE);
                bits
            })
            .collect::<Vec<_>>();
        bits.chunks(8).map(UInt8::from_bits_le).collect::<Vec<_>>()
    }
}
