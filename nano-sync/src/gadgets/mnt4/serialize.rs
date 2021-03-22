use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{G1Var, G2Var};
use ark_r1cs_std::prelude::{Boolean, ToBitsGadget};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

use crate::gadgets::mnt4::YToBitGadget;
use crate::utils::pad_point_bits;

/// A gadget that takes as input a G1 or G2 point and serializes it into a vector of Booleans.
pub struct SerializeGadget;

impl SerializeGadget {
    pub fn serialize_g1(
        cs: ConstraintSystemRef<MNT4Fr>,
        point: &G1Var,
    ) -> Result<Vec<Boolean<MNT4Fr>>, SynthesisError> {
        // Convert the point to affine coordinates.
        let aff_point = point.to_affine()?;

        // Get bits from the x coordinate.
        let x_bits = aff_point.x.to_bits_le()?;

        // Get one bit from the y coordinate.
        let y_bit = YToBitGadget::y_to_bit_g1(cs, &aff_point)?;

        // Get the infinity flag.
        let infinity_bit = aff_point.infinity;

        // Pad points and get *Big-Endian* representation.
        let bits = pad_point_bits::<MNT4Fr>(x_bits, y_bit, infinity_bit);

        Ok(bits)
    }

    pub fn serialize_g2(
        cs: ConstraintSystemRef<MNT4Fr>,
        point: &G2Var,
    ) -> Result<Vec<Boolean<MNT4Fr>>, SynthesisError> {
        // Convert the point to affine coordinates.
        let aff_point = point.to_affine()?;

        // Get bits from the x coordinate.
        let x_bits = aff_point.x.to_bits_le()?;

        // Get one bit from the y coordinate.
        let y_bit = YToBitGadget::y_to_bit_g2(cs, &aff_point)?;

        // Get the infinity flag.
        let infinity_bit = aff_point.infinity;

        // Pad points and get *Big-Endian* representation.
        let bits = pad_point_bits::<MNT4Fr>(x_bits, y_bit, infinity_bit);

        Ok(bits)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt4_753::Fr as MNT4Fr;
    use ark_mnt6_753::constraints::{G1Var, G2Var};
    use ark_mnt6_753::{G1Projective, G2Projective};
    use ark_r1cs_std::prelude::AllocVar;
    use ark_r1cs_std::R1CSVar;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};

    use nimiq_bls::utils::bytes_to_bits;
    use nimiq_nano_primitives::{serialize_g1_mnt6, serialize_g2_mnt6};

    use super::*;

    #[test]
    fn serialization_g1_mnt6_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random point.
        let g1_point = G1Projective::rand(rng);

        // Allocate the random inputs in the circuit.
        let g1_point_var = G1Var::new_witness(cs.clone(), || Ok(g1_point)).unwrap();

        // Serialize using the primitive version.
        let primitive_bytes = serialize_g1_mnt6(g1_point);
        let primitive_bits = bytes_to_bits(&primitive_bytes);

        // Serialize using the gadget version.
        let gadget_bits = SerializeGadget::serialize_g1(cs, &g1_point_var).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_bits.len(), gadget_bits.len());
        for i in 0..primitive_bits.len() {
            assert_eq!(primitive_bits[i], gadget_bits[i].value().unwrap());
        }
    }

    #[test]
    fn serialization_g2_mnt6_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fr>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random point.
        let g2_point = G2Projective::rand(rng);

        // Allocate the random inputs in the circuit.
        let g2_point_var = G2Var::new_witness(cs.clone(), || Ok(g2_point)).unwrap();

        // Serialize using the primitive version.
        let primitive_bytes = serialize_g2_mnt6(g2_point);
        let primitive_bits = bytes_to_bits(&primitive_bytes);

        // Serialize using the gadget version.
        let gadget_bits = SerializeGadget::serialize_g2(cs, &g2_point_var).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_bits.len(), gadget_bits.len());
        for i in 0..primitive_bits.len() {
            assert_eq!(primitive_bits[i], gadget_bits[i].value().unwrap());
        }
    }
}
