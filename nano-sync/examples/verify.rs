use std::fs::File;
use std::time::Instant;

use ark_crypto_primitives::SNARK;
use ark_groth16::{Groth16, Proof, VerifyingKey};
use ark_mnt6_753::MNT6_753;
use ark_serialize::CanonicalDeserialize;

use nimiq_nano_sync::primitives::vk_commitment;
use nimiq_nano_sync::utils::{bytes_to_bits, prepare_inputs};

fn main() {
    println!("====== Proof verification for Nano Sync initiated ======");
    let start = Instant::now();

    println!("Loading data from file");
    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/merger_wrapper.bin")).unwrap();

    let vk = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the proof from file.
    let mut file = File::open("proofs/proof_epoch_0.bin").unwrap();

    let proof = Proof::deserialize_unchecked(&mut file).unwrap();

    // Load the state from file.
    let mut file = File::open("proofs/state_epoch_0.bin").unwrap();

    let initial_state_commitment = Vec::<u8>::deserialize_unchecked(&mut file).unwrap();

    let final_state_commitment = Vec::<u8>::deserialize_unchecked(&mut file).unwrap();

    // Prepare the inputs.
    println!("Prepare inputs");

    let mut inputs = vec![];

    inputs.append(&mut prepare_inputs(bytes_to_bits(
        &initial_state_commitment,
    )));

    inputs.append(&mut prepare_inputs(bytes_to_bits(&final_state_commitment)));

    inputs.append(&mut prepare_inputs(bytes_to_bits(&vk_commitment(
        vk.clone(),
    ))));

    println!("Starting proof verification.");
    let result = Groth16::<MNT6_753>::verify(&vk, &inputs, &proof).unwrap();

    println!("Proof verification finished. It returned {}.", result);

    println!("====== Proof verification for Nano Sync finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());
}
