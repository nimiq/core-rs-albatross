use ark_ec::{pairing::Pairing, CurveGroup};
use ark_ff::Field;
use ark_groth16::{constraints::VerifyingKeyVar, VerifyingKey};
use ark_r1cs_std::{
    alloc::AllocVar, boolean::Boolean, eq::EqGadget, groups::GroupOpsBounds, pairing::PairingVar,
    uint8::UInt8,
};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use ark_serialize::CanonicalSerialize;

use crate::gadgets::serialize::SerializeGadget;

use super::vk_commitment::unwrap_or_dummy_vk;

type BasePrimeField<E> = <<<E as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField;

/// This gadget is meant to pass a verifying key by its serialized form.
pub struct SerializedVKGadget<E: Pairing, P: PairingVar<E, BasePrimeField<E>>> {
    // Public input: serialized vk
    pub serialized_vk: Vec<UInt8<BasePrimeField<E>>>,

    // Private input: actual vk
    pub vk: VerifyingKeyVar<E, P>,
}

impl<E: Pairing, P: PairingVar<E, BasePrimeField<E>>> SerializedVKGadget<E, P>
where
    P::G1Var: SerializeGadget<BasePrimeField<E>>,
    P::G2Var: SerializeGadget<BasePrimeField<E>>,
{
    /// Allocate gadget
    pub fn new(
        cs: ConstraintSystemRef<BasePrimeField<E>>,
        vk: Option<VerifyingKey<E>>,
        num_public_inputs: usize,
    ) -> Result<Self, SynthesisError> {
        let vk = unwrap_or_dummy_vk(vk, num_public_inputs)?;

        Ok(Self {
            serialized_vk: UInt8::new_input_vec(cs.clone(), &{
                let mut s = vec![];
                vk.serialize_compressed(&mut s)
                    .map_err(|_| SynthesisError::MalformedVerifyingKey)?;
                s
            })?,
            vk: VerifyingKeyVar::new_witness(cs, || Ok(vk))?,
        })
    }

    /// Calculates the verifying key commitment.
    pub fn verify(
        &self,
        cs: ConstraintSystemRef<BasePrimeField<E>>,
    ) -> Result<Boolean<BasePrimeField<E>>, SynthesisError>
    where
        for<'a> &'a P::G1Var: GroupOpsBounds<'a, E::G1, P::G1Var>,
    {
        // let pedersen_generators = DefaultPedersenParametersVar::new_constant(
        //     cs.clone(),
        //     pedersen_parameters().sub_window::<VkCommitmentWindow>(),
        // )?;

        // Initialize Boolean vector.
        let bytes = self.vk.serialize_compressed(cs)?;

        // Compare serializations.
        self.serialized_vk.is_eq(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use ark_ec::CurveGroup;
    use ark_groth16::VerifyingKey;
    use ark_mnt6_753::{
        constraints::PairingVar, Fq as MNT6Fq, G1Projective, G2Projective, MNT6_753,
    };
    use ark_r1cs_std::R1CSVar;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use nimiq_test_log::test;

    use super::*;

    #[test]
    fn serialized_vk_test() {
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
        let mut serialized_vk = vec![];
        vk.serialize_compressed(&mut serialized_vk).unwrap();

        // Evaluate vk commitment using the gadget version.
        let gadget_comm =
            SerializedVKGadget::<MNT6_753, PairingVar>::new(cs.clone(), Some(vk), 1).unwrap();

        // Compare the two versions bit by bit.
        assert_eq!(serialized_vk.len(), gadget_comm.serialized_vk.len());
        for i in 0..serialized_vk.len() {
            assert_eq!(
                serialized_vk[i],
                gadget_comm.serialized_vk[i].value().unwrap()
            );
        }

        assert!(gadget_comm.verify(cs.clone()).unwrap().value().unwrap());

        println!("Num constraints: {}", cs.num_constraints());
    }
}
