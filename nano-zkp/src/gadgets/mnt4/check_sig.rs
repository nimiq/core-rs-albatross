use ark_ec::ProjectiveCurve;
use ark_mnt4_753::Fr as MNT4Fr;
use ark_mnt6_753::constraints::{G1Var, G2Var, PairingVar};
use ark_mnt6_753::G1Projective;
use ark_r1cs_std::prelude::{AllocVar, Boolean, EqGadget, PairingVar as PV};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};

/// This gadget checks that a given BLS signature is correct. But it only works when given the hash
/// point, it does not have functionality to transform a message into the hash point.
pub struct CheckSigGadget;

impl CheckSigGadget {
    /// This function checks the correctness of a BLS signature for a given message and public key.
    /// It implements the following verification formula:
    /// e(gen, sig) = e(pk, hash)
    pub fn check_signature(
        cs: ConstraintSystemRef<MNT4Fr>,
        public_key: &G1Var,
        hash_point: &G2Var,
        signature: &G2Var,
    ) -> Result<Boolean<MNT4Fr>, SynthesisError> {
        // Prepare the public key elliptic curve point.
        let pub_key_p_var = PairingVar::prepare_g1(public_key)?;

        // Prepare the hash elliptic curve point.
        let hash_p_var = PairingVar::prepare_g2(hash_point)?;

        // Prepare the aggregated signature elliptic curve point.
        let sig_p_var = PairingVar::prepare_g2(signature)?;

        // Prepare the generator elliptic curve point.
        let generator = G1Var::new_constant(cs, G1Projective::prime_subgroup_generator())?;

        let generator_p_var = PairingVar::prepare_g1(&generator)?;

        // Calculate the pairing for the left hand side of the verification equation.
        // e(gen, sig)
        let pairing1_var = PairingVar::pairing(generator_p_var, sig_p_var)?;

        // Calculate the pairing for the right hand side of the verification equation.
        // e(pk, hash)
        let pairing2_var = PairingVar::pairing(pub_key_p_var, hash_p_var)?;

        // Check the equality of both sides of the verification equation.
        pairing1_var.is_eq(&pairing2_var)
    }
}
