use ark_ec::{pairing::Pairing, CurveGroup};
use ark_ff::Field;
use ark_groth16::VerifyingKey;
use ark_r1cs_std::{
    boolean::Boolean, eq::EqGadget, groups::GroupOpsBounds, pairing::PairingVar, uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use nimiq_zkp_primitives::{vk_commitment, vks_commitment};

use crate::gadgets::{
    pedersen::{PedersenHashGadget, PedersenParametersVar},
    serialize::SerializeGadget,
};

use super::vk_commitment::VkCommitmentWindow;

type BasePrimeField<E> = <<<E as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField;

/// An enum to hold vks from two different curves.
pub enum RecursiveVK<P1: Pairing, P2: Pairing> {
    Curve1(VerifyingKey<P1>),
    Curve2(VerifyingKey<P2>),
}

/// This gadget is meant to calculate a commitment in-circuit over a list of other commitments.
pub struct VKsCommitmentGadget<E: Pairing> {
    // Public input: commitment over all vk commitments
    pub main_commitment: Vec<UInt8<BasePrimeField<E>>>,

    // Private input: sub commitments
    pub vk_commitments: Vec<Vec<UInt8<BasePrimeField<E>>>>,
}

impl<E: Pairing> VKsCommitmentGadget<E> {
    /// Allocate gadget
    pub fn new(
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        vk_commitments: Vec<Option<[u8; 95]>>,
    ) -> Result<Self, SynthesisError> {
        let vk_commitments = vk_commitments
            .iter()
            .map(|commitment| commitment.unwrap_or_else(|| [0u8; 95]))
            .collect::<Vec<_>>();
        let commitment = vks_commitment(&vk_commitments);
        Ok(Self {
            main_commitment: UInt8::new_input_vec(cs.clone(), &commitment)?,
            vk_commitments: vk_commitments
                .iter()
                .map(|commitment| UInt8::new_witness_vec(cs.clone(), commitment))
                .collect::<Result<Vec<_>, SynthesisError>>()?,
        })
    }

    /// Allocate gadget
    pub fn from_vks<OtherE: Pairing>(
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        // We also need the number of public inputs in case of `None`.
        mut vks: Vec<Option<RecursiveVK<E, OtherE>>>,
    ) -> Result<Self, SynthesisError> {
        let vk_commitments = vks
            .drain(..)
            .map(|vk| match vk {
                Some(RecursiveVK::Curve1(vk)) => Some(vk_commitment(vk)),
                Some(RecursiveVK::Curve2(vk)) => Some(vk_commitment(vk)),
                None => None,
            })
            .collect::<Vec<_>>();
        Self::new(cs, vk_commitments)
    }

    /// Calculates the verifying key commitment.
    pub fn verify<P: PairingVar<E, BasePrimeField<E>>>(
        &self,
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        pedersen_generators: &PedersenParametersVar<E::G1, P::G1Var>,
    ) -> Result<Boolean<BasePrimeField<E>>, SynthesisError>
    where
        P::G1Var: SerializeGadget<BasePrimeField<E>>,
        for<'a> &'a P::G1Var: GroupOpsBounds<'a, E::G1, P::G1Var>,
    {
        // let pedersen_generators = DefaultPedersenParametersVar::new_constant(
        //     cs.clone(),
        //     pedersen_parameters().sub_window::<VkCommitmentWindow>(),
        // )?;

        // Initialize Boolean vector.
        let mut bytes = vec![];
        for commitment in self.vk_commitments.iter() {
            bytes.extend(commitment.clone());
        }

        // Calculate the Pedersen hash.
        let hash =
            PedersenHashGadget::<_, _, VkCommitmentWindow>::evaluate(&bytes, &pedersen_generators)?;

        // Serialize the Pedersen hash.
        let serialized_bytes = hash.serialize_compressed(cs)?;

        self.main_commitment.is_eq(&serialized_bytes)
    }
}

#[cfg(test)]
mod tests {
    use ark_ec::CurveGroup;
    use ark_groth16::VerifyingKey;
    use ark_mnt4_753::{
        G1Projective as MNT4G1Projective, G2Projective as MNT4G2Projective, MNT4_753,
    };
    use ark_mnt6_753::{
        constraints::PairingVar, Fq as MNT6Fq, G1Projective as MNT6G1Projective,
        G2Projective as MNT6G2Projective, MNT6_753,
    };
    use ark_r1cs_std::{prelude::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use nimiq_test_log::test;
    use nimiq_zkp_primitives::{pedersen_parameters_mnt6, vk_commitment};

    use crate::gadgets::mnt6::DefaultPedersenParametersVar;

    use super::*;

    #[test]
    fn vks_commitment_test() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create verifying keys.
        let mut vk1 = VerifyingKey::<MNT6_753>::default();
        vk1.alpha_g1 = MNT6G1Projective::rand(rng).into_affine();
        vk1.beta_g2 = MNT6G2Projective::rand(rng).into_affine();
        vk1.gamma_g2 = MNT6G2Projective::rand(rng).into_affine();
        vk1.delta_g2 = MNT6G2Projective::rand(rng).into_affine();
        vk1.gamma_abc_g1 = vec![
            MNT6G1Projective::rand(rng).into_affine(),
            MNT6G1Projective::rand(rng).into_affine(),
        ];

        let mut vk2 = VerifyingKey::<MNT4_753>::default();
        vk2.alpha_g1 = MNT4G1Projective::rand(rng).into_affine();
        vk2.beta_g2 = MNT4G2Projective::rand(rng).into_affine();
        vk2.gamma_g2 = MNT4G2Projective::rand(rng).into_affine();
        vk2.delta_g2 = MNT4G2Projective::rand(rng).into_affine();
        vk2.gamma_abc_g1 = vec![
            MNT4G1Projective::rand(rng).into_affine(),
            MNT4G1Projective::rand(rng).into_affine(),
        ];

        let vks = vec![
            Some(RecursiveVK::Curve1(vk1.clone())),
            Some(RecursiveVK::Curve2(vk2.clone())),
        ];

        // Evaluate vk commitment using the gadget version.
        let gadget_comm = VKsCommitmentGadget::from_vks(cs.clone(), vks).unwrap();

        // Verify commitment.
        let pedersen_generators = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt6().sub_window::<VkCommitmentWindow>(),
        )
        .unwrap();
        assert!(gadget_comm
            .verify::<PairingVar>(cs.clone(), &pedersen_generators)
            .unwrap()
            .value()
            .unwrap());

        // Compare the two versions bit by bit.
        let vk_comm1 = vk_commitment(vk1);
        assert_eq!(vk_comm1.len(), gadget_comm.vk_commitments[0].len());
        for i in 0..vk_comm1.len() {
            assert_eq!(
                vk_comm1[i],
                gadget_comm.vk_commitments[0][i].value().unwrap()
            );
        }

        let vk_comm2 = vk_commitment(vk2);
        assert_eq!(vk_comm2.len(), gadget_comm.vk_commitments[1].len());
        for i in 0..vk_comm2.len() {
            assert_eq!(
                vk_comm2[i],
                gadget_comm.vk_commitments[1][i].value().unwrap()
            );
        }

        let vks_comm = vks_commitment(&vec![vk_comm1, vk_comm2]);
        assert_eq!(vks_comm.len(), gadget_comm.main_commitment.len());
        for i in 0..vks_comm.len() {
            assert_eq!(vks_comm[i], gadget_comm.main_commitment[i].value().unwrap());
        }

        println!("Num constraints: {}", cs.num_constraints());
    }
}
