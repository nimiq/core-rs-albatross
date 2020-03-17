use algebra::sw6::Fr as SW6Fr;
use r1cs_core::SynthesisError;
use r1cs_std::bls12_377::{
    Fq12Gadget, G1Gadget, G1PreparedGadget, G2Gadget, G2PreparedGadget, PairingGadget,
};
use r1cs_std::{eq::EqGadget, fields::FieldGadget, pairing::PairingGadget as PG};

pub struct CheckSigGadget;

impl CheckSigGadget {
    /// Implements signature aggregation from https://crypto.stanford.edu/%7Edabo/pubs/papers/aggreg.pdf .
    pub fn check_signatures<CS: r1cs_core::ConstraintSystem<SW6Fr>>(
        mut cs: CS,
        public_keys: &[G2Gadget],
        generator: &G2Gadget,
        signature: &G1Gadget,
        hash_points: &[G1Gadget],
    ) -> Result<(), SynthesisError> {
        assert_eq!(
            hash_points.len(),
            public_keys.len(),
            "One hash point per public key is required"
        );
        assert!(hash_points.len() > 1, "Min one message is required");

        let sig_p_var: G1PreparedGadget = PairingGadget::prepare_g1(cs.ns(|| "sig_p"), &signature)?;
        let generator_p_var: G2PreparedGadget =
            PairingGadget::prepare_g2(cs.ns(|| "generator"), &generator)?;

        let mut hash_p_vars = vec![];
        for (i, hash_point) in hash_points.iter().enumerate() {
            let hash_p_var: G1PreparedGadget =
                PairingGadget::prepare_g1(cs.ns(|| format!("hash_p {}", i)), &hash_point)?;
            hash_p_vars.push(hash_p_var);
        }

        let mut pub_key_p_vars = vec![];
        for (i, public_key) in public_keys.iter().enumerate() {
            let pub_key_p_var: G2PreparedGadget =
                PairingGadget::prepare_g2(cs.ns(|| format!("pub_key_p {}", i)), &public_key)?;
            pub_key_p_vars.push(pub_key_p_var);
        }

        let pairing1_var: Fq12Gadget =
            PairingGadget::pairing(cs.ns(|| "sig pairing"), sig_p_var, generator_p_var.clone())?;

        let mut pairings2_var: Vec<Fq12Gadget> = vec![];
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

        let mut pairing2_var = pairings2_var.pop().unwrap();
        for (i, p) in pairings2_var.iter().enumerate() {
            pairing2_var.mul_in_place(cs.ns(|| format!("add pk {}", i)), p)?;
        }

        pairing1_var.enforce_equal(cs.ns(|| "pairing equality"), &pairing2_var)?;
        Ok(())
    }
}
