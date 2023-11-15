use ark_ec::{pairing::Pairing, CurveGroup};
use ark_ff::Field;
use ark_groth16::{constraints::VerifyingKeyVar, VerifyingKey};
use ark_r1cs_std::{
    alloc::AllocVar, boolean::Boolean, eq::EqGadget, groups::GroupOpsBounds, pairing::PairingVar,
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use ark_serialize::CanonicalSerialize;
use nimiq_pedersen_generators::DefaultWindow;
use nimiq_zkp_primitives::vk_commitment;

use crate::gadgets::{
    pedersen::{PedersenHashGadget, PedersenParametersVar},
    serialize::SerializeGadget,
};

type BasePrimeField<E> = <<<E as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField;

pub(crate) fn unwrap_or_dummy_vk<E: Pairing>(
    vk: Option<VerifyingKey<E>>,
    num_public_inputs: usize,
) -> Result<VerifyingKey<E>, SynthesisError> {
    let vk = vk.unwrap_or_else(|| {
        let mut vk = VerifyingKey::<E>::default();
        for _ in 0..num_public_inputs + 1 {
            vk.gamma_abc_g1.push(Default::default());
        }
        vk
    });

    // Check validity
    if vk.gamma_abc_g1.len() != num_public_inputs + 1 {
        return Err(SynthesisError::MalformedVerifyingKey);
    }

    Ok(vk)
}

pub enum VKRepr<E: Pairing, P: PairingVar<E, BasePrimeField<E>>> {
    Native(VerifyingKeyVar<E, P>),
    Serialized(Vec<UInt8<BasePrimeField<E>>>),
}

impl<E: Pairing, P: PairingVar<E, BasePrimeField<E>>> VKRepr<E, P> {
    pub fn native(&self) -> Option<&VerifyingKeyVar<E, P>> {
        match self {
            VKRepr::Native(vk) => Some(vk),
            VKRepr::Serialized(_) => None,
        }
    }

    pub fn serialized(&self) -> Option<&[UInt8<BasePrimeField<E>>]> {
        match self {
            VKRepr::Native(_) => None,
            VKRepr::Serialized(s) => Some(s),
        }
    }
}

/// This gadget is meant to calculate a commitment in-circuit for a verifying key of a SNARK.
/// Since the verifying key might not be compatible with the current curve, it supports opening
/// the commitment to the serialization only. Then, on a recursive circuit, the verifying key can
/// be matched to its corresponding serialization.
pub struct VKCommitmentGadget<E: Pairing, P: PairingVar<E, BasePrimeField<E>>> {
    // Public input: vk commitment
    pub vk_commitment: Vec<UInt8<BasePrimeField<E>>>,

    // Private input: actual vk
    pub vk: VKRepr<E, P>,
}

pub type VkCommitmentWindow = DefaultWindow;

impl<E: Pairing, P: PairingVar<E, BasePrimeField<E>>> VKCommitmentGadget<E, P>
where
    P::G1Var: SerializeGadget<BasePrimeField<E>>,
    P::G2Var: SerializeGadget<BasePrimeField<E>>,
{
    /// Allocate gadget
    pub fn new_native(
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        vk: Option<VerifyingKey<E>>,
        num_public_inputs: usize,
    ) -> Result<Self, SynthesisError> {
        let vk = unwrap_or_dummy_vk(vk, num_public_inputs)?;
        Ok(Self {
            vk_commitment: UInt8::new_input_vec(cs.clone(), &vk_commitment(vk.clone()))?,
            vk: VKRepr::Native(VerifyingKeyVar::new_witness(cs, || Ok(vk))?),
        })
    }

    /// Allocate gadget
    pub fn new_serialized<OtherE: Pairing>(
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        vk: Option<VerifyingKey<OtherE>>,
        num_public_inputs: usize,
    ) -> Result<Self, SynthesisError> {
        // Create verifying key
        let vk = unwrap_or_dummy_vk(vk, num_public_inputs)?;

        // Serialize
        let mut s = vec![];
        vk.serialize_compressed(&mut s)
            .map_err(|_| SynthesisError::MalformedVerifyingKey)?;

        Ok(Self {
            vk_commitment: UInt8::new_input_vec(cs.clone(), &vk_commitment(vk))?,
            vk: VKRepr::Serialized(UInt8::new_witness_vec(cs, &s)?),
        })
    }

    /// Calculates the verifying key commitment.
    pub fn verify(
        &self,
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        pedersen_generators: &PedersenParametersVar<E::G1, P::G1Var>,
    ) -> Result<Boolean<BasePrimeField<E>>, SynthesisError>
    where
        for<'a> &'a P::G1Var: GroupOpsBounds<'a, E::G1, P::G1Var>,
    {
        // let pedersen_generators = DefaultPedersenParametersVar::new_constant(
        //     cs.clone(),
        //     pedersen_parameters().sub_window::<VkCommitmentWindow>(),
        // )?;

        // Initialize Boolean vector.
        let bytes;
        let bytes_ref = match self.vk {
            VKRepr::Native(ref vk) => {
                bytes = vk.serialize_compressed(cs.clone())?;
                &bytes
            }
            VKRepr::Serialized(ref v) => v,
        };

        // Calculate the Pedersen hash.
        let hash = PedersenHashGadget::<_, _, VkCommitmentWindow>::evaluate(
            bytes_ref,
            pedersen_generators,
        )?;

        // Serialize the Pedersen hash.
        let serialized_bytes = hash.serialize_compressed(cs)?;

        self.vk_commitment.is_eq(&serialized_bytes)
    }
}

#[cfg(test)]
mod tests {
    use ark_ec::CurveGroup;
    use ark_groth16::VerifyingKey;
    use ark_mnt6_753::{
        constraints::PairingVar, Fq as MNT6Fq, G1Projective, G2Projective, MNT6_753,
    };
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use nimiq_test_log::test;
    use nimiq_zkp_primitives::{pedersen_parameters_mnt6, vk_commitment};

    use crate::gadgets::mnt6::DefaultPedersenParametersVar;

    use super::*;

    #[test]
    fn vk_commitment_native_test() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create verifying key.
        let mut vk = VerifyingKey::<MNT6_753>::default();
        vk.alpha_g1 = G1Projective::rand(rng).into_affine();
        vk.beta_g2 = G2Projective::rand(rng).into_affine();
        vk.gamma_g2 = G2Projective::rand(rng).into_affine();
        vk.delta_g2 = G2Projective::rand(rng).into_affine();
        vk.gamma_abc_g1 = vec![
            G1Projective::rand(rng).into_affine(),
            G1Projective::rand(rng).into_affine(),
        ];

        // Evaluate vk commitment using the primitive version.
        let primitive_comm = vk_commitment(vk.clone());

        // Allocate vk commitment using the gadget version.
        let gadget_comm =
            VKCommitmentGadget::<MNT6_753, PairingVar>::new_native(cs.clone(), Some(vk.clone()), 1)
                .unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_comm.len(), gadget_comm.vk_commitment.len());
        for i in 0..primitive_comm.len() {
            assert_eq!(
                primitive_comm[i],
                gadget_comm.vk_commitment[i].value().unwrap()
            );
        }

        // Verify commitment.
        let pedersen_generators = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt6().sub_window::<VkCommitmentWindow>(),
        )
        .unwrap();
        assert!(gadget_comm
            .verify(cs.clone(), &pedersen_generators)
            .unwrap()
            .value()
            .unwrap());

        println!("Num constraints: {}", cs.num_constraints());
    }

    #[test]
    fn vk_commitment_serialized_test() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create verifying key.
        let mut vk = VerifyingKey::<MNT6_753>::default();
        vk.alpha_g1 = G1Projective::rand(rng).into_affine();
        vk.beta_g2 = G2Projective::rand(rng).into_affine();
        vk.gamma_g2 = G2Projective::rand(rng).into_affine();
        vk.delta_g2 = G2Projective::rand(rng).into_affine();
        vk.gamma_abc_g1 = vec![
            G1Projective::rand(rng).into_affine(),
            G1Projective::rand(rng).into_affine(),
        ];

        // Evaluate vk commitment using the primitive version.
        let primitive_comm = vk_commitment(vk.clone());
        let mut serialized_vk = vec![];
        vk.serialize_compressed(&mut serialized_vk).unwrap();

        // Allocate vk commitment using the gadget version.
        let gadget_comm = VKCommitmentGadget::<MNT6_753, PairingVar>::new_serialized(
            cs.clone(),
            Some(vk.clone()),
            1,
        )
        .unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(primitive_comm.len(), gadget_comm.vk_commitment.len());
        for i in 0..primitive_comm.len() {
            assert_eq!(
                primitive_comm[i],
                gadget_comm.vk_commitment[i].value().unwrap()
            );
        }

        // Compare the two versions bit by bit.
        assert_eq!(primitive_comm.len(), gadget_comm.vk_commitment.len());
        for i in 0..primitive_comm.len() {
            assert_eq!(
                primitive_comm[i],
                gadget_comm.vk_commitment[i].value().unwrap()
            );
        }
        assert_eq!(
            serialized_vk.len(),
            gadget_comm.vk.serialized().unwrap().len()
        );
        for i in 0..serialized_vk.len() {
            assert_eq!(
                serialized_vk[i],
                gadget_comm.vk.serialized().unwrap()[i].value().unwrap()
            );
        }

        // Verify commitment.
        let pedersen_generators = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt6().sub_window::<VkCommitmentWindow>(),
        )
        .unwrap();
        assert!(gadget_comm
            .verify(cs.clone(), &pedersen_generators)
            .unwrap()
            .value()
            .unwrap());

        println!("Num constraints: {}", cs.num_constraints());
    }
}
