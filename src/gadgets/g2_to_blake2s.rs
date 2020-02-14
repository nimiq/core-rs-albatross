use algebra::curves::bls12_377::Bls12_377Parameters;
use algebra::fields::sw6::Fr as SW6Fr;
use r1cs_core::SynthesisError;
use r1cs_std::{
    bits::ToBitsGadget, boolean::Boolean, groups::curves::short_weierstrass::bls12::G2Gadget,
};

use crate::gadgets::y_to_bit::YToBitGadget;
use crate::gadgets::{hash_to_bits, pad_point_bits, reverse_inner_byte_order};
use algebra::fields::bls12_377::FqParameters;
use crypto_primitives::prf::blake2s::constraints::blake2s_gadget;

pub struct G2ToBlake2sGadget {}

impl G2ToBlake2sGadget {
    pub fn hash_from_g2<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        g2: &G2Gadget<Bls12_377Parameters>,
    ) -> Result<Vec<Boolean>, SynthesisError> {
        // Convert g2 to bits before hashing.
        let serialized_bits: Vec<Boolean> = g2.x.to_bits(cs.ns(|| "bits"))?;
        let greatest_bit =
            YToBitGadget::<Bls12_377Parameters>::y_to_bit_g2(cs.ns(|| "y to bit"), g2)?;

        // Pad points and get *Big-Endian* representation.
        let serialized_bits = pad_point_bits::<FqParameters>(serialized_bits, greatest_bit);

        // Prepare order of booleans for blake2s (it doesn't expect Big-Endian).
        let serialized_bits = reverse_inner_byte_order(&serialized_bits);

        // Hash serialized bits.
        let h0 = blake2s_gadget(cs.ns(|| "h0 from serialized bits"), &serialized_bits)?;
        let h0_bits = hash_to_bits(h0);
        Ok(h0_bits)
    }
}
