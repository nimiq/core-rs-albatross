use std::fs::File;
use std::io;
use std::time::Instant;

use ark_groth16::Proof;
use ark_serialize::CanonicalDeserialize;

use nimiq_nano_sync::constants::EPOCH_LENGTH;
use nimiq_nano_sync::utils::create_test_blocks;
use nimiq_nano_sync::NanoZKP;

/// Verifies a proof for a chain of election blocks. The random parameters generation uses always
/// the same seed, so it will always generate the same data (validators, signatures, etc).
/// This function will simply print the verification result.
/// Run this example with `cargo run --all-features --release --example verify`.
fn main() {
    // Ask user for the number of epochs.
    println!("Enter the number of epochs to verify:");

    let mut data = String::new();

    io::stdin()
        .read_line(&mut data)
        .expect("Couldn't read user input.");

    let number_epochs: u64 = data.trim().parse().expect("Couldn't read user input.");

    println!("====== Generating random inputs ======");

    // Get initial random parameters.
    let (initial_pks, initial_header_hash, _, _, _) = create_test_blocks(0);

    // Get final random parameters.
    let (final_pks, final_header_hash, _, _, _) = create_test_blocks(number_epochs as u64);

    // Load the proof from file.
    let mut file = File::open(format!("proofs/proof_epoch_{}.bin", number_epochs)).unwrap();

    let proof = Proof::deserialize_unchecked(&mut file).unwrap();

    println!("====== Proof verification for Nano Sync initiated ======");

    let start = Instant::now();

    // Verify proof.
    let result = NanoZKP::verify(
        0,
        initial_header_hash,
        initial_pks,
        EPOCH_LENGTH * number_epochs as u32,
        final_header_hash,
        final_pks,
        proof,
    )
    .unwrap();

    println!("Proof verification finished. It returned {}.", result);

    println!("====== Proof verification for Nano Sync finished ======");
    println!("Total time elapsed: {:?}", start.elapsed());
}
