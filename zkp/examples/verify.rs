use std::{fs::File, io, time::Instant};

use ark_groth16::Proof;
use ark_serialize::CanonicalDeserialize;
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_zkp::{verify::verify, ZKP_VERIFYING_KEY};
use nimiq_zkp_circuits::utils::create_test_blocks;

/// Verifies a proof for a chain of election blocks. The random parameters generation uses always
/// the same seed, so it will always generate the same data (validators, signatures, etc).
/// This function will simply print the verification result.
/// Run this example with `cargo run --release --example verify`.
fn main() {
    ZKP_VERIFYING_KEY.init_with_network_id(NetworkId::DevAlbatross);

    // Ask user for the number of epochs.
    println!("Enter the number of epochs to verify:");

    let mut data = String::new();

    io::stdin()
        .read_line(&mut data)
        .expect("Couldn't read user input.");

    let number_epochs: u64 = data.trim().parse().expect("Couldn't read user input.");

    println!("====== Generating random inputs ======");

    // Get initial random parameters.
    let (_, genesis_header_hash, genesis_pk_tree_root, _, _, _) = create_test_blocks(0);

    // Get final random parameters.
    let (_, final_header_hash, final_pk_tree_root, _, _, _) = create_test_blocks(number_epochs);

    // Load the proof from file.
    let mut file = File::open(format!("proofs/proof_epoch_{number_epochs}.bin")).unwrap();

    let proof = Proof::deserialize_uncompressed_unchecked(&mut file).unwrap();

    println!("====== Proof verification for Nano Sync initiated ======");

    let start = Instant::now();

    // Verify proof.
    let result = verify(
        0,
        genesis_header_hash,
        &genesis_pk_tree_root,
        Policy::blocks_per_epoch() * number_epochs as u32,
        final_header_hash,
        &final_pk_tree_root,
        proof,
        &ZKP_VERIFYING_KEY,
    )
    .unwrap();

    println!("Proof verification finished. It returned {result}.");

    println!("====== Proof verification for Nano Sync finished ======");
    println!("Total time elapsed: {:?}", start.elapsed());
}
