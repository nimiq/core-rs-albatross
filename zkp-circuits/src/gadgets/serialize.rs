use ark_ec::{pairing::Pairing, short_weierstrass::SWCurveConfig, CurveGroup};
use ark_ff::{Field, PrimeField};
use ark_groth16::constraints::VerifyingKeyVar;
use ark_r1cs_std::{
    groups::curves::short_weierstrass::{AffineVar, ProjectiveVar},
    pairing::PairingVar,
    prelude::{FieldOpsBounds, FieldVar, ToBitsGadget},
    uint64::UInt64,
    uint8::UInt8,
    ToBytesGadget,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use ark_serialize::buffer_byte_size;

use super::y_to_bit::YToBitGadget;

type BasePrimeField<E> = <<<E as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField;
/// A gadget that takes as input a G1 or G2 point and serializes it into a vector of Booleans.
pub trait SerializeGadget<F: Field> {
    fn serialize_compressed(
        &self,
        cs: ConstraintSystemRef<F>,
    ) -> Result<Vec<UInt8<F>>, SynthesisError>;
}

impl<F: Field> SerializeGadget<F> for UInt64<F> {
    fn serialize_compressed(
        &self,
        _cs: ConstraintSystemRef<F>,
    ) -> Result<Vec<UInt8<F>>, SynthesisError> {
        self.to_bytes()
    }
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

impl<E: Pairing, P: PairingVar<E, BasePrimeField<E>>> SerializeGadget<BasePrimeField<E>>
    for VerifyingKeyVar<E, P>
where
    P::G1Var: SerializeGadget<BasePrimeField<E>>,
    P::G2Var: SerializeGadget<BasePrimeField<E>>,
{
    fn serialize_compressed(
        &self,
        cs: ConstraintSystemRef<BasePrimeField<E>>,
    ) -> Result<Vec<UInt8<BasePrimeField<E>>>, SynthesisError> {
        // Initialize Boolean vector.
        let mut bytes = vec![];

        // Serialize the verifying key into bits.
        // Alpha G1
        bytes.extend(self.alpha_g1.serialize_compressed(cs.clone())?);

        // Beta G2
        bytes.extend(self.beta_g2.serialize_compressed(cs.clone())?);

        // Gamma G2
        bytes.extend(self.gamma_g2.serialize_compressed(cs.clone())?);

        // Delta G2
        bytes.extend(self.delta_g2.serialize_compressed(cs.clone())?);

        // Gamma ABC G1
        let len = self.gamma_abc_g1.len() as u64;
        bytes.extend(UInt64::constant(len).serialize_compressed(cs.clone())?);
        for i in 0..self.gamma_abc_g1.len() {
            bytes.extend(self.gamma_abc_g1[i].serialize_compressed(cs.clone())?);
        }

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests_mnt4 {
    use ark_groth16::VerifyingKey;
    use ark_mnt4_753::{
        constraints::{G1Var, G2Var, PairingVar},
        Fq as MNT4Fq, G1Projective, G2Projective, MNT4_753,
    };
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_serialize::CanonicalSerialize;
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

    #[test]
    fn serialization_vk_mnt4_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random vk.
        let mut vk = VerifyingKey::<MNT4_753>::default();
        vk.alpha_g1 = G1Projective::rand(rng).into_affine();
        vk.beta_g2 = G2Projective::rand(rng).into_affine();
        vk.gamma_g2 = G2Projective::rand(rng).into_affine();
        vk.delta_g2 = G2Projective::rand(rng).into_affine();
        vk.gamma_abc_g1 = vec![
            G1Projective::rand(rng).into_affine(),
            G1Projective::rand(rng).into_affine(),
        ];

        // Allocate the random vk in the circuit.
        let vk_var =
            VerifyingKeyVar::<MNT4_753, PairingVar>::new_witness(cs.clone(), || Ok(vk.clone()))
                .unwrap();

        // Serialize using the primitive version.
        let mut primitive_bytes = vec![];
        vk.serialize_compressed(&mut primitive_bytes).unwrap();

        // Serialize using the gadget version.
        let gadget_bytes = vk_var.serialize_compressed(cs).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_bytes.len(), gadget_bytes.len());
        for i in 0..primitive_bytes.len() {
            assert_eq!(primitive_bytes[i], gadget_bytes[i].value().unwrap());
        }
    }
}

#[cfg(test)]
mod tests_mnt6 {
    use ark_groth16::VerifyingKey;
    use ark_mnt6_753::{
        constraints::{G1Var, G2Var, PairingVar},
        Fq as MNT6Fq, G1Projective, G2Projective, MNT6_753,
    };
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_serialize::CanonicalSerialize;
    use ark_std::{test_rng, UniformRand};
    use nimiq_test_log::test;
    use nimiq_zkp_primitives::{serialize_g1_mnt6, serialize_g2_mnt6};

    use super::*;

    #[test]
    fn serialization_g1_mnt6_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

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
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

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

    #[test]
    fn serialization_vk_mnt6_works() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create random vk.
        let mut vk = VerifyingKey::<MNT6_753>::default();
        vk.alpha_g1 = G1Projective::rand(rng).into_affine();
        vk.beta_g2 = G2Projective::rand(rng).into_affine();
        vk.gamma_g2 = G2Projective::rand(rng).into_affine();
        vk.delta_g2 = G2Projective::rand(rng).into_affine();
        vk.gamma_abc_g1 = vec![
            G1Projective::rand(rng).into_affine(),
            G1Projective::rand(rng).into_affine(),
        ];

        // Allocate the random vk in the circuit.
        let vk_var =
            VerifyingKeyVar::<MNT6_753, PairingVar>::new_witness(cs.clone(), || Ok(vk.clone()))
                .unwrap();

        // Serialize using the primitive version.
        let mut primitive_bytes = vec![];
        vk.serialize_compressed(&mut primitive_bytes).unwrap();

        // Serialize using the gadget version.
        let gadget_bytes = vk_var.serialize_compressed(cs).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_bytes.len(), gadget_bytes.len());
        for i in 0..primitive_bytes.len() {
            assert_eq!(primitive_bytes[i], gadget_bytes[i].value().unwrap());
        }
    }
}
