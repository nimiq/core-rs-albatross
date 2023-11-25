use std::marker::PhantomData;

use ark_crypto_primitives::crh::pedersen::Window;
use ark_ec::{pairing::Pairing, CurveGroup};
use ark_ff::Field;
use ark_groth16::{constraints::VerifyingKeyVar, VerifyingKey};
use ark_r1cs_std::{
    alloc::AllocVar, eq::EqGadget, groups::GroupOpsBounds, pairing::PairingVar, uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use nimiq_pedersen_generators::DefaultWindow;

use crate::gadgets::{
    pedersen::{PedersenHashGadget, PedersenParametersVar},
    serialize::SerializeGadget,
};

type BasePrimeField<E> = <<<E as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField;

pub type VkCommitmentWindow = DefaultWindow;

/// This gadget is meant to calculate a commitment in-circuit for a verifying key of a SNARK.
/// Since the verifying key might not be compatible with the current curve, it supports opening
/// the commitment to the serialization only. Then, on a recursive circuit, the verifying key can
/// be matched to its corresponding serialization.
pub struct VkCommitmentGadget<E: Pairing, P: PairingVar<E, BasePrimeField<E>>, W: Window> {
    // Public input: vk commitment
    pub vk_commitment: Vec<UInt8<BasePrimeField<E>>>,

    // Private input: actual vk
    pub vk: VerifyingKeyVar<E, P>,

    _window: PhantomData<W>,
}

impl<E: Pairing, P: PairingVar<E, BasePrimeField<E>>, W: Window> VkCommitmentGadget<E, P, W>
where
    P::G1Var: SerializeGadget<BasePrimeField<E>>,
    P::G2Var: SerializeGadget<BasePrimeField<E>>,
{
    /// Allocate gadget and verify
    pub fn new_and_verify(
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        vk: &VerifyingKey<E>,
        commitment: Vec<UInt8<BasePrimeField<E>>>,
        pedersen_generators: &PedersenParametersVar<E::G1, P::G1Var>,
    ) -> Result<Self, SynthesisError>
    where
        for<'a> &'a P::G1Var: GroupOpsBounds<'a, E::G1, P::G1Var>,
    {
        let gadget = Self::new(cs.clone(), vk, commitment)?;
        gadget.verify(cs, pedersen_generators)?;
        Ok(gadget)
    }

    /// Allocate gadget
    pub fn new(
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        vk: &VerifyingKey<E>,
        commitment: Vec<UInt8<BasePrimeField<E>>>,
    ) -> Result<Self, SynthesisError> {
        Ok(Self {
            vk_commitment: commitment,
            vk: VerifyingKeyVar::new_witness(cs, || Ok(vk))?,
            _window: PhantomData,
        })
    }

    /// Calculates the verifying key commitment.
    pub fn verify(
        &self,
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        pedersen_generators: &PedersenParametersVar<E::G1, P::G1Var>,
    ) -> Result<(), SynthesisError>
    where
        for<'a> &'a P::G1Var: GroupOpsBounds<'a, E::G1, P::G1Var>,
    {
        // Initialize Boolean vector.
        let bytes = self.vk.serialize_compressed(cs.clone())?;

        // Calculate the Pedersen hash.
        let hash = PedersenHashGadget::<_, _, W>::evaluate(&bytes, pedersen_generators)?;

        // Serialize the Pedersen hash.
        let serialized_bytes = hash.serialize_compressed(cs.clone())?;

        self.vk_commitment.enforce_equal(&serialized_bytes)
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

    use super::*;
    use crate::gadgets::mnt6::DefaultPedersenParametersVar;

    #[test]
    fn vk_commitment_test() {
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
        let primitive_comm = vk_commitment(&vk);

        // Allocate vk commitment using the gadget version.
        let comm = UInt8::new_input_vec(cs.clone(), &primitive_comm).unwrap();
        let pedersen_generators = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt6().sub_window::<VkCommitmentWindow>(),
        )
        .unwrap();
        let gadget_comm =
            VkCommitmentGadget::<MNT6_753, PairingVar, VkCommitmentWindow>::new_and_verify(
                cs.clone(),
                &vk,
                comm,
                &pedersen_generators,
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

        assert!(cs.is_satisfied().unwrap());

        println!("Num constraints: {}", cs.num_constraints());
    }
}
