use ark_bls12_381::{
    g1::Config as BlsG1Config, Bls12_381, Config as BlsConfig, Fq as BlsFq, G1Projective as BlsG1,
    G2Projective as BlsG2,
};
use ark_ec::{bls12::Bls12Config, pairing::Pairing, Group};
use ark_ff::UniformRand;
use ark_mnt6_753::Fq as MNT6Fq;
use ark_r1cs_std::{
    fields::fp2::NonNativeFp2Var,
    fields::nonnative::NonNativeFieldVar,
    groups::curves::short_weierstrass::GenericProjectiveVar,
    pairing::bls12::PairingVar as BlsPairing,
    pairing::PairingVar,
    prelude::{AllocVar, EqGadget},
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use ark_std::Zero;
use rand::Rng;

fn secret_key<C: Pairing, R: Rng + ?Sized>(rng: &mut R) -> C::ScalarField {
    let mut sk = C::ScalarField::rand(rng);
    loop {
        if !sk.is_zero() {
            break;
        }
        sk = C::ScalarField::rand(rng);
    }
    sk
}

fn sign<C: Pairing>(sk: C::ScalarField, msg: C::G1) -> C::G1 {
    msg * sk
}

fn public_key<C: Pairing>(sk: C::ScalarField) -> C::G2 {
    C::G2::generator() * sk
}

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
        let sk = secret_key::<Bls12_381, _>(rng);
        let message = BlsG1::rand(rng);
        let pk = public_key::<Bls12_381>(sk);
        let signature = sign::<Bls12_381>(sk, message);

        TestCircuit::new(pk, signature, message)
    }
}

type Fp2G<P> = NonNativeFp2Var<<P as Bls12Config>::Fp2Config, MNT6Fq>;
type BlsG2Var = GenericProjectiveVar<<BlsConfig as Bls12Config>::G2Config, MNT6Fq, Fp2G<BlsConfig>>;
type BlsG1Var = GenericProjectiveVar<BlsG1Config, MNT6Fq, NonNativeFieldVar<BlsFq, MNT6Fq>>;
type PairingV = BlsPairing<BlsConfig, MNT6Fq, NonNativeFieldVar<BlsFq, MNT6Fq>>;

impl ConstraintSynthesizer<MNT6Fq> for TestCircuit {
    /// This function generates the constraints for the circuit.
    fn generate_constraints(self, cs: ConstraintSystemRef<MNT6Fq>) -> Result<(), SynthesisError> {
        // Witnesses

        let public_key = BlsG2Var::new_witness(cs.clone(), || Ok(self.public_key))?;

        let signature = BlsG1Var::new_witness(cs.clone(), || Ok(self.signature))?;

        let message = BlsG1Var::new_witness(cs.clone(), || Ok(self.message))?;

        // Prepare the public key elliptic curve point.
        let pub_key_p_var = PairingV::prepare_g2(&public_key)?;

        // Prepare the hash elliptic curve point.
        let hash_p_var = PairingV::prepare_g1(&message)?;

        // Prepare the aggregated signature elliptic curve point.
        let sig_p_var = PairingV::prepare_g1(&signature)?;

        // Prepare the generator elliptic curve point.
        let generator = BlsG2Var::new_constant(cs, BlsG2::generator())?;

        let generator_p_var = PairingV::prepare_g2(&generator)?;

        // Calculate the pairing for the left hand side of the verification equation.
        // e(sig, gen)
        let pairing1_var = PairingV::pairing(sig_p_var, generator_p_var)?;

        // Calculate the pairing for the right hand side of the verification equation.
        // e(hash, pk)
        let pairing2_var = PairingV::pairing(hash_p_var, pub_key_p_var)?;

        // Check the equality of both sides of the verification equation.
        pairing1_var.enforce_equal(&pairing2_var)?;

        Ok(())
    }
}
