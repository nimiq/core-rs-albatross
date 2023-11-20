use ark_ec::{pairing::Pairing, CurveGroup};
use ark_ff::Field;
use ark_r1cs_std::{eq::EqGadget, groups::GroupOpsBounds, pairing::PairingVar, uint8::UInt8};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use nimiq_zkp_primitives::pedersen::DefaultPedersenParameters95;

use super::vk_commitment::VkCommitmentWindow;
use crate::gadgets::{
    pedersen::{PedersenHashGadget, PedersenParametersVar},
    serialize::SerializeGadget,
};

type BasePrimeField<E> = <<<E as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField;

/// This gadget is meant to calculate a commitment in-circuit over a list of other commitments.
pub struct VksCommitmentGadget<E: Pairing + DefaultPedersenParameters95> {
    // Public input: commitment over all vk commitments
    pub main_commitment: Vec<UInt8<BasePrimeField<E>>>,

    // Private input: sub commitments
    pub vk_commitments: Vec<Vec<UInt8<BasePrimeField<E>>>>,
}

impl<E: Pairing + DefaultPedersenParameters95> VksCommitmentGadget<E> {
    /// Allocate gadget and verify
    pub fn new_and_verify<P: PairingVar<E, BasePrimeField<E>>>(
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        vk_commitments: Vec<Option<[u8; 95]>>,
        main_commitment: Vec<UInt8<BasePrimeField<E>>>,
        pedersen_generators: &PedersenParametersVar<E::G1, P::G1Var>,
    ) -> Result<Self, SynthesisError>
    where
        P::G1Var: SerializeGadget<BasePrimeField<E>>,
        for<'a> &'a P::G1Var: GroupOpsBounds<'a, E::G1, P::G1Var>,
    {
        let gadget = Self::new(cs.clone(), vk_commitments, main_commitment)?;
        gadget.verify::<P>(cs, pedersen_generators)?;
        Ok(gadget)
    }

    /// Allocate gadget
    pub fn new(
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        vk_commitments: Vec<Option<[u8; 95]>>,
        main_commitment: Vec<UInt8<BasePrimeField<E>>>,
    ) -> Result<Self, SynthesisError> {
        let vk_commitments = vk_commitments
            .iter()
            .map(|commitment| commitment.unwrap_or_else(|| [0u8; 95]))
            .collect::<Vec<_>>();
        Ok(Self {
            main_commitment,
            vk_commitments: vk_commitments
                .iter()
                .map(|commitment| UInt8::new_witness_vec(cs.clone(), commitment))
                .collect::<Result<Vec<_>, SynthesisError>>()?,
        })
    }

    /// Calculates the verifying key commitment.
    pub fn verify<P: PairingVar<E, BasePrimeField<E>>>(
        &self,
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        pedersen_generators: &PedersenParametersVar<E::G1, P::G1Var>,
    ) -> Result<(), SynthesisError>
    where
        P::G1Var: SerializeGadget<BasePrimeField<E>>,
        for<'a> &'a P::G1Var: GroupOpsBounds<'a, E::G1, P::G1Var>,
    {
        // Initialize Boolean vector.
        let mut bytes = vec![];
        for commitment in self.vk_commitments.iter() {
            bytes.extend(commitment.clone());
        }

        // Calculate the Pedersen hash.
        let hash =
            PedersenHashGadget::<_, _, VkCommitmentWindow>::evaluate(&bytes, pedersen_generators)?;

        // Serialize the Pedersen hash.
        let serialized_bytes = hash.serialize_compressed(cs)?;

        self.main_commitment.enforce_equal(&serialized_bytes)
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
    use nimiq_zkp_primitives::{pedersen_parameters_mnt6, vk_commitment, vks_commitment};

    use super::*;
    use crate::gadgets::mnt6::DefaultPedersenParametersVar;

    #[test]
    fn vks_commitment_test() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create verifying keys.
        let mut vk1 = VerifyingKey::<MNT6_753>::default();
        vk1.alpha_g1 = G1Projective::rand(rng).into_affine();
        vk1.beta_g2 = G2Projective::rand(rng).into_affine();
        vk1.gamma_g2 = G2Projective::rand(rng).into_affine();
        vk1.delta_g2 = G2Projective::rand(rng).into_affine();
        vk1.gamma_abc_g1 = vec![
            G1Projective::rand(rng).into_affine(),
            G1Projective::rand(rng).into_affine(),
        ];

        let mut vk2 = VerifyingKey::<MNT6_753>::default();
        vk2.alpha_g1 = G1Projective::rand(rng).into_affine();
        vk2.beta_g2 = G2Projective::rand(rng).into_affine();
        vk2.gamma_g2 = G2Projective::rand(rng).into_affine();
        vk2.delta_g2 = G2Projective::rand(rng).into_affine();
        vk2.gamma_abc_g1 = vec![
            G1Projective::rand(rng).into_affine(),
            G1Projective::rand(rng).into_affine(),
        ];

        let vk_comm1 = vk_commitment(&vk1);
        let vk_comm2 = vk_commitment(&vk2);
        let vk_comms = vec![Some(vk_comm1.clone()), Some(vk_comm2.clone())];
        let vks_comm = vks_commitment::<MNT6_753>(&vec![vk_comm1, vk_comm2]);

        // Evaluate vk commitment using the gadget version.
        let pedersen_generators = DefaultPedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt6().sub_window::<VkCommitmentWindow>(),
        )
        .unwrap();
        let comm = UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &vks_comm).unwrap();
        let gadget_comm = VksCommitmentGadget::new_and_verify::<PairingVar>(
            cs.clone(),
            vk_comms,
            comm,
            &pedersen_generators,
        )
        .unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(vk_comm1.len(), gadget_comm.vk_commitments[0].len());
        for i in 0..vk_comm1.len() {
            assert_eq!(
                vk_comm1[i],
                gadget_comm.vk_commitments[0][i].value().unwrap()
            );
        }

        assert_eq!(vk_comm2.len(), gadget_comm.vk_commitments[1].len());
        for i in 0..vk_comm2.len() {
            assert_eq!(
                vk_comm2[i],
                gadget_comm.vk_commitments[1][i].value().unwrap()
            );
        }

        assert_eq!(vks_comm.len(), gadget_comm.main_commitment.len());
        for i in 0..vks_comm.len() {
            assert_eq!(vks_comm[i], gadget_comm.main_commitment[i].value().unwrap());
        }

        assert!(cs.is_satisfied().unwrap());

        println!("Num constraints: {}", cs.num_constraints());
    }
}
