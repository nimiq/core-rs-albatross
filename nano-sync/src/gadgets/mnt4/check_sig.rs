use ark_ec::ProjectiveCurve;
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{G1Var, G2Var, PairingVar};
use ark_mnt6_753::G2Projective;
use ark_r1cs_std::prelude::{AllocVar, EqGadget, PairingVar as PV};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

/// This gadget checks that a given BLS signature is correct. But it only works when given the hash
/// point, it does not have functionality to transform a message into the hash point.
pub struct CheckSigGadget;

impl CheckSigGadget {
    /// This function checks the correctness of a BLS signature for a given message and public key.
    /// It implements the following verification formula:
    /// e(sig, gen) = e(hash, pk)
    pub fn check_signature(
        cs: ConstraintSystemRef<MNT4Fr>,
        public_key: &G2Var,
        hash_point: &G1Var,
        signature: &G1Var,
    ) -> Result<(), SynthesisError> {
        // Prepare all the public key elliptic curve point.
        let pub_key_p_var = PairingVar::prepare_g2(public_key)?;

        // Prepare all the hash elliptic curve point.
        let hash_p_var = PairingVar::prepare_g1(hash_point)?;

        // Prepare the aggregated signature elliptic curve point.
        let sig_p_var = PairingVar::prepare_g1(&signature)?;

        // Prepare the generator elliptic curve point.
        let generator = G2Var::new_constant(cs.clone(), G2Projective::prime_subgroup_generator())?;

        let generator_p_var = PairingVar::prepare_g2(&generator)?;

        // Calculate the pairing for the left hand side of the verification equation.
        // e(sig, gen)
        let pairing1_var = PairingVar::pairing(sig_p_var, generator_p_var)?;

        // Calculate the pairing for the right hand side of the verification equation.
        // e(hash, pk)
        let pairing2_var = PairingVar::pairing(hash_p_var, pub_key_p_var)?;

        // Enforce the equality of both sides of the verification equation.
        pairing1_var.enforce_equal(&pairing2_var)?;

        Ok(())
    }
}
