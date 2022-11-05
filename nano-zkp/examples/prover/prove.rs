use std::fs::{DirBuilder, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ark_groth16::Proof;
use ark_serialize::CanonicalSerialize;

use nimiq_nano_zkp::utils::create_test_blocks;
use nimiq_nano_zkp::NanoZKP;

/// Generates a proof for a chain of election blocks. The random parameters generation uses always
/// the same seed, so it will always generate the same data (validators, signatures, etc).
/// This function will simply output a proof for the final epoch and store it in file.
/// Run this example with `cargo run --all-features --release --example prove`.
fn main() {
    // Ask user for the number of epochs.
    println!("Enter the number of epochs to prove:");

    let mut data = String::new();

    io::stdin()
        .read_line(&mut data)
        .expect("Couldn't read user input.");

    let number_epochs: u64 = data.trim().parse().expect("Couldn't read user input.");

    println!("====== Proof generation for Nano Sync initiated ======");

    let start = Instant::now();

    let mut genesis_state_commitment = vec![];
    let mut genesis_data = None;
    let mut proof = Proof::default();

    for i in 0..number_epochs {
        // Get random parameters.
        let (initial_pks, initial_header_hash, final_pks, block, genesis_state_commitment_opt) =
            create_test_blocks(i as u64);

        // Create genesis data.
        if i == 0 {
            genesis_state_commitment = genesis_state_commitment_opt.unwrap();
        } else {
            genesis_data = Some((proof, genesis_state_commitment.clone()))
        };

        println!("Proving epoch {}", i + 1);

        // Generate proof.
        proof = NanoZKP::prove(
            initial_pks,
            initial_header_hash,
            final_pks.clone(),
            block,
            genesis_data.clone(),
            true,
            true,
            &PathBuf::new(), // use the current directory
        )
        .unwrap();

        // Save proof to file.
        if !Path::new("proofs/").is_dir() {
            DirBuilder::new().create("proofs/").unwrap();
        }

        let mut file = File::create(format!("proofs/proof_epoch_{}.bin", i + 1)).unwrap();

        proof.serialize_unchecked(&mut file).unwrap();

        file.sync_all().unwrap();
    }

    println!("====== Proof generation for Nano Sync finished ======");
    println!("Total time elapsed: {:?}", start.elapsed());
}
