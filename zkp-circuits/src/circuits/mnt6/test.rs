use ark_bls12_381::{
    g1::Config as BlsG1Config, g2::Config as BlsG2Config, Fq as BlsFq, G1Projective as BlsG1,
    G2Projective as BlsG2,
};
use ark_crypto_primitives::snark::SNARKGadget;
use ark_ff::UniformRand;
use ark_groth16::{
    constraints::{Groth16VerifierGadget, ProofVar, VerifyingKeyVar},
    Proof, VerifyingKey,
};
use ark_mnt6_753::{
    constraints::{G2Var, PairingVar},
    Fq as MNT6Fq, G1Affine, G2Affine, G2Projective, MNT6_753,
};
use ark_r1cs_std::{
    fields::nonnative::NonNativeFieldVar,
    groups::curves::short_weierstrass::ProjectiveVar,
    prelude::{AllocVar, Boolean, EqGadget, UInt32, UInt8},
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use nimiq_block::MacroBlock;
use nimiq_primitives::policy::Policy;
use nimiq_zkp_primitives::PEDERSEN_PARAMETERS;
use rand::Rng;

use super::pk_tree_node::{hash_g2, PkInnerNodeWindow};
use crate::{
    blake2s::evaluate_blake2s,
    gadgets::{
        mnt6::{DefaultPedersenParametersVar, MacroBlockGadget},
        recursive_input::RecursiveInputVar,
    },
};

#[derive(Clone)]
pub struct TestCircuit {
    // Private
    pub public_key: BlsG2,
    pub signature: BlsG1,
    pub message: BlsG1,
}

impl TestCircuit {
    pub fn new(public_key: BlsG2, signature: BlsG1, message: BlsG1) -> Self {
        Self {
            public_key,
            signature,
            message,
        }
    }

    pub fn rand<R: Rng + ?Sized>(rng: &mut R) -> Self {
        TestCircuit::new(BlsG2::rand(rng), BlsG1::rand(rng), BlsG1::rand(rng))
    }
}

impl ConstraintSynthesizer<MNT6Fq> for TestCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT6Fq>) -> Result<(), SynthesisError> {
        // Witnesses
        // let public_key_var =
        //     ProjectiveVar::<BlsG2Config, NonNativeFieldVar<MNT6Fq, BlsFq>>::new_witness(
        //         cs,
        //         || Ok(self.public_key),
        //     )?;

        let signature =
            ProjectiveVar::<BlsG1Config, NonNativeFieldVar<MNT6Fq, BlsFq>>::new_witness(
                cs,
                || Ok(self.signature),
            )?;
        Ok(())
    }
}
