use ark_ec::CurveConfig;
use ark_ff::{Field, PrimeField};
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::{
    constraints::{G1Var, G2Var},
    g2::Config as G2Config,
};
use ark_r1cs_std::{
    prelude::{ToBitsGadget, ToBytesGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use ark_serialize::buffer_byte_size;

use crate::gadgets::mnt4::YToBitGadget;

/// A gadget that takes as input a G1 or G2 point and serializes it into a vector of Booleans.
pub struct SerializeGadget;

impl SerializeGadget {
    pub fn serialize_g1(
        cs: ConstraintSystemRef<MNT4Fr>,
        point: &G1Var,
    ) -> Result<Vec<UInt8<MNT4Fr>>, SynthesisError> {
        // Convert the point to affine coordinates.
        let aff_point = point.to_affine()?;

        // Get bits from the x coordinate.
        let mut bytes = aff_point.x.to_bytes()?;

        // Truncate unnecessary bytes.
        let output_byte_size = buffer_byte_size(MNT4Fr::MODULUS_BIT_SIZE as usize + 2);
        bytes.truncate(output_byte_size);

        // Get the y coordinate parity flag.
        let y_bit = YToBitGadget::y_to_bit_g1(cs, &aff_point)?;

        // Add y-bit and infinity flag.
        let mut last_byte = bytes.pop().unwrap().to_bits_le()?;
        last_byte[6] = aff_point.infinity;
        last_byte[7] = y_bit;
        bytes.push(UInt8::from_bits_le(&last_byte));

        Ok(bytes)
    }

    pub fn serialize_g2(
        cs: ConstraintSystemRef<MNT4Fr>,
        point: &G2Var,
    ) -> Result<Vec<UInt8<MNT4Fr>>, SynthesisError> {
        // Convert the point to affine coordinates.
        let aff_point = point.to_affine()?;

        // Get bits from the x coordinate.
        let x_bytes = aff_point.x.to_bytes()?;

        // Truncate unnecessary bytes for each extension degree of x.
        let extension_degree = <G2Config as CurveConfig>::BaseField::extension_degree() as usize;
        assert_eq!(x_bytes.len() % extension_degree, 0);

        let output_byte_size = buffer_byte_size(MNT4Fr::MODULUS_BIT_SIZE as usize + 2);
        let mut bytes = Vec::with_capacity(extension_degree * output_byte_size);
        for coordinate in x_bytes.chunks(x_bytes.len() / extension_degree) {
            bytes.extend_from_slice(&coordinate[..output_byte_size]);
        }

        // Get the y coordinate parity flag.
        let y_bit = YToBitGadget::y_to_bit_g2(cs, &aff_point)?;

        // Add y-bit and infinity flag.
        let mut last_byte = bytes.pop().unwrap().to_bits_le()?;
        last_byte[6] = aff_point.infinity;
        last_byte[7] = y_bit;
        bytes.push(UInt8::from_bits_le(&last_byte));

        Ok(bytes)
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

    use nimiq_test_log::test;
    use nimiq_zkp_primitives::{serialize_g1_mnt6, serialize_g2_mnt6};

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
        let primitive_bytes = serialize_g1_mnt6(&g1_point);

        // Serialize using the gadget version.
        let gadget_bytes = SerializeGadget::serialize_g1(cs, &g1_point_var).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_bytes.len(), gadget_bytes.len());
        for i in 0..primitive_bytes.len() {
            assert_eq!(primitive_bytes[i], gadget_bytes[i].value().unwrap());
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
        let primitive_bytes = serialize_g2_mnt6(&g2_point);

        // Serialize using the gadget version.
        let gadget_bytes = SerializeGadget::serialize_g2(cs, &g2_point_var).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_bytes.len(), gadget_bytes.len());
        for i in 0..primitive_bytes.len() {
            assert_eq!(primitive_bytes[i], gadget_bytes[i].value().unwrap());
        }
    }
}
