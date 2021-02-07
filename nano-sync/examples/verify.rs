use std::fs::File;
use std::time::Instant;

use ark_crypto_primitives::SNARK;
use ark_ec::ProjectiveCurve;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt6_753::{Fr as MNT6Fr, G1Projective as G1MNT6, G2Projective as G2MNT6, MNT6_753};
use ark_serialize::CanonicalDeserialize;
use ark_std::ops::MulAssign;
use ark_std::{test_rng, UniformRand};

use nimiq_nano_sync::constants::VALIDATOR_SLOTS;
use nimiq_nano_sync::primitives::{state_commitment};

use nimiq_nano_sync::NanoZKP;

fn main() {
    println!("====== Generating random inputs ======");
    let rng = &mut test_rng();

    // Create key pairs for all the initial validators.
    let mut initial_sks = vec![];
    let mut initial_pks = vec![];

    for _ in 0..VALIDATOR_SLOTS {
        let sk = MNT6Fr::rand(rng);
        let mut pk = G2MNT6::prime_subgroup_generator();
        pk.mul_assign(sk);
        initial_sks.push(sk);
        initial_pks.push(pk);
    }

    // Create key pairs for all the final validators.
    let mut final_sks = vec![];
    let mut final_pks = vec![];

    for _ in 0..VALIDATOR_SLOTS {
        let sk = MNT6Fr::rand(rng);
        let mut pk = G2MNT6::prime_subgroup_generator();
        pk.mul_assign(sk);
        final_sks.push(sk);
        final_pks.push(pk);
    }

    // Calculate the commitments.
    let initial_state_commitment = state_commitment(0, initial_pks);

    let final_state_commitment = state_commitment(0, final_pks);

    // Load the proof from file.
    let mut file = File::open("proofs/proof_epoch_0.bin").unwrap();

    let proof = Proof::deserialize_unchecked(&mut file).unwrap();

    println!("====== Proof verification for Nano Sync initiated ======");
    let start = Instant::now();

    let result = NanoZKP::verify(initial_state_commitment, final_state_commitment, proof).unwrap();

    println!("Proof verification finished. It returned {}.", result);

    println!("====== Proof verification for Nano Sync finished ======");
    println!("Total time elapsed: {:?}", start.elapsed());
}
