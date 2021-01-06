use ark_mnt6_753::constraints::{G1Var, G2Var, PairingVar};
use ark_r1cs_std::eq::EqGadget;
use ark_r1cs_std::prelude::PairingVar as PV;
use ark_relations::r1cs::SynthesisError;

/// This gadget checks that a given BLS signature is correct. But it only works when given the hash
/// point, it does not have functionality to transform a message into the hash point.
pub struct CheckSigGadget;

impl CheckSigGadget {
    /// This function checks the correctness of an aggregated signature for n different messages
    /// from n different public keys. It implements the signature aggregation scheme from:
    /// https://crypto.stanford.edu/%7Edabo/pubs/papers/aggreg.pdf
    /// It implements the following verification formula:
    /// e(sig, gen) = e(hash_1, pk_1) * e(hash_2, pk_2) * ... * e(hash_n, pk_n)
    pub fn check_signatures(
        public_keys: &[&G2Var],
        hash_points: &[&G1Var],
        signature: &G1Var,
        generator: &G2Var,
    ) -> Result<(), SynthesisError> {
        // Make some initial sanity checks.
        assert_eq!(
            hash_points.len(),
            public_keys.len(),
            "One hash point per public key is required"
        );

        assert!(hash_points.len() > 1, "Min one message is required");

        // Prepare all the public key elliptic curve points.
        let mut pub_key_p_vars = vec![];

        for public_key in public_keys {
            let pub_key_p_var = PairingVar::prepare_g2(public_key)?;
            pub_key_p_vars.push(pub_key_p_var);
        }

        // Prepare all the hash elliptic curve points.
        let mut hash_p_vars = vec![];

        for hash_point in hash_points {
            let hash_p_var = PairingVar::prepare_g1(hash_point)?;
            hash_p_vars.push(hash_p_var);
        }

        // Prepare the aggregated signature elliptic curve point.
        let sig_p_var = PairingVar::prepare_g1(&signature)?;

        // Prepare the generator elliptic curve point.
        let generator_p_var = PairingVar::prepare_g2(&generator)?;

        // Calculate the pairing for the left hand side of the verification equation.
        // e(sig, gen)
        let pairing1_var = PairingVar::pairing(sig_p_var, generator_p_var)?;

        // Calculate the pairings for the right hand side of the verification equation.
        // e(hash_1, pk_1), e(hash_2, pk_2), ... , e(hash_n, pk_n)
        let mut pairings2_var = vec![];

        for i in 0..hash_p_vars.len() {
            let pairing2_var =
                PairingVar::pairing(hash_p_vars[i].clone(), pub_key_p_vars[i].clone())?;
            pairings2_var.push(pairing2_var);
        }

        // Multiply all of the pairings of the right hand side of the verification equation.
        //  e(hash_1, pk_1)*e(hash_2, pk_2)*...*e(hash_n, pk_n)
        let mut pairing2_var = pairings2_var.pop().unwrap();

        for p in pairings2_var {
            pairing2_var = pairing2_var * p;
        }

        // Enforce the equality of both sides of the verification equation.
        pairing1_var.enforce_equal(&pairing2_var)?;

        Ok(())
    }
}
