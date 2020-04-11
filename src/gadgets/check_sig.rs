use algebra::mnt4_753::Fr as MNT4Fr;
use r1cs_core::SynthesisError;
use r1cs_std::mnt6_753::{
    G1Gadget, G1PreparedGadget, G2Gadget, G2PreparedGadget, PairingGadget,
};
use r1cs_std::{eq::EqGadget, fields::FieldGadget, pairing::PairingGadget as PG};

/// This gadget checks that a given BLS signature is correct. But it only works if given the hash
/// point(s), it does not have functionality to transform a message into the hash point(s).
pub struct CheckSigGadget;

impl CheckSigGadget {
    /// This function the correctness of an aggregated signature for n different messages from n
    /// different public keys. It implements the signature aggregation scheme from:
    /// https://crypto.stanford.edu/%7Edabo/pubs/papers/aggreg.pdf
    /// It implements the following verification formula:
    /// e(sig, gen) = e(hash_1, pk_1)*e(hash_2, pk_2)*...*e(hash_n, pk_n)
    pub fn check_signatures<CS: r1cs_core::ConstraintSystem<MNT4Fr>>(
        mut cs: CS,
        public_keys: &[G2Gadget],
        generator: &G2Gadget,
        signature: &G1Gadget,
        hash_points: &[G1Gadget],
    ) -> Result<(), SynthesisError> {
        // Make some initial sanity checks.
        assert_eq!(
            hash_points.len(),
            public_keys.len(),
            "One hash point per public key is required"
        );
        assert!(hash_points.len() > 1, "Min one message is required");

        // Prepare the aggregated signature elliptic curve point.
        let sig_p_var: G1PreparedGadget = PairingGadget::prepare_g1(cs.ns(|| "sig_p"), &signature)?;

        // Prepare the generator elliptic curve point.
        let generator_p_var: G2PreparedGadget =
            PairingGadget::prepare_g2(cs.ns(|| "generator"), &generator)?;

        // Prepare all the hash elliptic curve points.
        let mut hash_p_vars = vec![];
        for (i, hash_point) in hash_points.iter().enumerate() {
            let hash_p_var: G1PreparedGadget =
                PairingGadget::prepare_g1(cs.ns(|| format!("hash_p {}", i)), &hash_point)?;
            hash_p_vars.push(hash_p_var);
        }

        // Prepare all the public key elliptic curve points.
        let mut pub_key_p_vars = vec![];
        for (i, public_key) in public_keys.iter().enumerate() {
            let pub_key_p_var: G2PreparedGadget =
                PairingGadget::prepare_g2(cs.ns(|| format!("pub_key_p {}", i)), &public_key)?;
            pub_key_p_vars.push(pub_key_p_var);
        }

        // Calculate the pairing for the left hand side of the verification equation.
        // e(sig, gen)
        let pairing1_var =
            PairingGadget::pairing(cs.ns(|| "sig pairing"), sig_p_var, generator_p_var.clone())?;

        // Calculates the pairings for the right hand side of the verification equation.
        // e(hash_1, pk_1), e(hash_2, pk_2), ... , e(hash_n, pk_n)
        let mut pairings2_var = vec![];
        for (i, (hash_p_var, pub_key_p_var)) in hash_p_vars
            .drain(..)
            .zip(pub_key_p_vars.drain(..))
            .enumerate()
        {
            let pairing2_var = PairingGadget::pairing(
                cs.ns(|| format!("pub pairing {}", i)),
                hash_p_var,
                pub_key_p_var,
            )?;
            pairings2_var.push(pairing2_var);
        }

        // Multiply all of the pairings of the right hand side of the verification equation.
        //  e(hash_1, pk_1)*e(hash_2, pk_2)*...*e(hash_n, pk_n)
        let mut pairing2_var = pairings2_var.pop().unwrap();
        for (i, p) in pairings2_var.iter().enumerate() {
            pairing2_var.mul_in_place(cs.ns(|| format!("add pk {}", i)), p)?;
        }

        // Enforce the equality of both sides of the verification equation.
        pairing1_var.enforce_equal(cs.ns(|| "pairing equality"), &pairing2_var)?;
        Ok(())
    }
}
