use ark_ec::short_weierstrass::SWCurveConfig;
use ark_ff::{Field, PrimeField};
use ark_r1cs_std::{
    groups::curves::short_weierstrass::{AffineVar, ProjectiveVar},
    prelude::{FieldOpsBounds, FieldVar, ToBitsGadget},
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use ark_serialize::buffer_byte_size;

use super::y_to_bit::YToBitGadget;

/// A gadget that takes as input a G1 or G2 point and serializes it into a vector of Booleans.
pub trait SerializeGadget<F: Field> {
    fn serialize_compressed(
        &self,
        cs: ConstraintSystemRef<F>,
    ) -> Result<Vec<UInt8<F>>, SynthesisError>;
}

impl<P: SWCurveConfig, F: FieldVar<P::BaseField, <P::BaseField as Field>::BasePrimeField>>
    SerializeGadget<<P::BaseField as Field>::BasePrimeField> for AffineVar<P, F>
where
    for<'a> &'a F: FieldOpsBounds<'a, P::BaseField, F>,
    Self: YToBitGadget<<P::BaseField as Field>::BasePrimeField>,
{
    fn serialize_compressed(
        &self,
        cs: ConstraintSystemRef<<P::BaseField as Field>::BasePrimeField>,
    ) -> Result<Vec<UInt8<<P::BaseField as Field>::BasePrimeField>>, SynthesisError> {
        // Get bits from the x coordinate.
        let x_bytes = self.x.to_bytes()?;

        // Truncate unnecessary bytes for each extension degree of x.
        let extension_degree = P::BaseField::extension_degree() as usize;
        assert_eq!(x_bytes.len() % extension_degree, 0);

        let output_byte_size = buffer_byte_size(
            <P::BaseField as Field>::BasePrimeField::MODULUS_BIT_SIZE as usize + 2,
        );
        let mut bytes = Vec::with_capacity(extension_degree * output_byte_size);
        for coordinate in x_bytes.chunks(x_bytes.len() / extension_degree) {
            bytes.extend_from_slice(&coordinate[..output_byte_size]);
        }

        // Get the y coordinate parity flag.
        let y_bit = self.y_to_bit(cs)?;

        // Add y-bit and infinity flag.
        let mut last_byte = bytes.pop().unwrap().to_bits_le()?;
        last_byte[6] = self.infinity.clone();
        last_byte[7] = y_bit;
        bytes.push(UInt8::from_bits_le(&last_byte));

        Ok(bytes)
    }
}

impl<P: SWCurveConfig, F: FieldVar<P::BaseField, <P::BaseField as Field>::BasePrimeField>>
    SerializeGadget<<P::BaseField as Field>::BasePrimeField> for ProjectiveVar<P, F>
where
    for<'a> &'a F: FieldOpsBounds<'a, P::BaseField, F>,
    AffineVar<P, F>: SerializeGadget<<P::BaseField as Field>::BasePrimeField>,
{
    fn serialize_compressed(
        &self,
        cs: ConstraintSystemRef<<P::BaseField as Field>::BasePrimeField>,
    ) -> Result<Vec<UInt8<<P::BaseField as Field>::BasePrimeField>>, SynthesisError> {
        let affine = self.to_affine()?;

        affine.serialize_compressed(cs)
    }
}

#[cfg(test)]
mod tests_mnt4 {
    use ark_mnt4_753::{
        constraints::{G1Var, G2Var},
        Fq as MNT4Fq, G1Projective, G2Projective,
    };
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use nimiq_test_log::test;
    use nimiq_zkp_primitives::{serialize_g1_mnt4, serialize_g2_mnt4};

    use super::*;

    #[test]
    fn serialization_g1_mnt4_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random point.
        let g1_point = G1Projective::rand(rng);

        // Allocate the random inputs in the circuit.
        let g1_point_var = G1Var::new_witness(cs.clone(), || Ok(g1_point)).unwrap();

        // Serialize using the primitive version.
        let primitive_bytes = serialize_g1_mnt4(&g1_point);

        // Serialize using the gadget version.
        let gadget_bytes = g1_point_var.serialize_compressed(cs).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_bytes.len(), gadget_bytes.len());
        for i in 0..primitive_bytes.len() {
            assert_eq!(primitive_bytes[i], gadget_bytes[i].value().unwrap());
        }
    }

    #[test]
    fn serialization_g2_mnt4_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random point.
        let g2_point = G2Projective::rand(rng);

        // Allocate the random inputs in the circuit.
        let g2_point_var = G2Var::new_witness(cs.clone(), || Ok(g2_point)).unwrap();

        // Serialize using the primitive version.
        let primitive_bytes = serialize_g2_mnt4(&g2_point);

        // Serialize using the gadget version.
        let gadget_bytes = g2_point_var.serialize_compressed(cs).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_bytes.len(), gadget_bytes.len());
        for i in 0..primitive_bytes.len() {
            assert_eq!(
                primitive_bytes[i],
                gadget_bytes[i].value().unwrap(),
                "Mismatch in byte {}",
                i
            );
        }
    }
}

#[cfg(test)]
mod tests_mnt6 {
    use ark_mnt6_753::{
        constraints::{G1Var, G2Var},
        Fq as MNT4Fq, G1Projective, G2Projective,
    };
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use nimiq_test_log::test;
    use nimiq_zkp_primitives::{serialize_g1_mnt6, serialize_g2_mnt6};

    use super::*;

    #[test]
    fn serialization_g1_mnt6_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random point.
        let g1_point = G1Projective::rand(rng);

        // Allocate the random inputs in the circuit.
        let g1_point_var = G1Var::new_witness(cs.clone(), || Ok(g1_point)).unwrap();

        // Serialize using the primitive version.
        let primitive_bytes = serialize_g1_mnt6(&g1_point);

        // Serialize using the gadget version.
        let gadget_bytes = g1_point_var.serialize_compressed(cs).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_bytes.len(), gadget_bytes.len());
        for i in 0..primitive_bytes.len() {
            assert_eq!(primitive_bytes[i], gadget_bytes[i].value().unwrap());
        }
    }

    #[test]
    fn serialization_g2_mnt6_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random point.
        let g2_point = G2Projective::rand(rng);

        // Allocate the random inputs in the circuit.
        let g2_point_var = G2Var::new_witness(cs.clone(), || Ok(g2_point)).unwrap();

        // Serialize using the primitive version.
        let primitive_bytes = serialize_g2_mnt6(&g2_point);

        // Serialize using the gadget version.
        let gadget_bytes = g2_point_var.serialize_compressed(cs).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_bytes.len(), gadget_bytes.len());
        for i in 0..primitive_bytes.len() {
            assert_eq!(
                primitive_bytes[i],
                gadget_bytes[i].value().unwrap(),
                "Mismatch in byte {}",
                i
            );
        }
    }
}
