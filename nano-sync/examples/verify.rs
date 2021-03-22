use std::fs::File;
use std::time::Instant;

use ark_groth16::Proof;
use ark_serialize::CanonicalDeserialize;

use nimiq_nano_sync::constants::EPOCH_LENGTH;
use nimiq_nano_sync::utils::create_test_blocks;
use nimiq_nano_sync::NanoZKP;

const NUMBER_EPOCHS: usize = 2;
const SEED: u64 = 12370426996209291122;

/// Verifies a proof for a chain of election blocks. The random parameters generation uses always
/// the same seed, so it will always generate the same data (validators, signatures, etc).
/// This function will simply print the verification result.
fn main() {
    println!("====== Generating random inputs ======");

    // Get initial random parameters.
    let (initial_pks, initial_header_hash, _, _, _) = create_test_blocks(SEED, 0);

    // Get final random parameters.
    let (final_pks, final_header_hash, _, _, _) = create_test_blocks(SEED, NUMBER_EPOCHS as u64);

    // Load the proof from file.
    let mut file = File::open(format!("proofs/proof_epoch_{}.bin", NUMBER_EPOCHS)).unwrap();

    let proof = Proof::deserialize_unchecked(&mut file).unwrap();

    println!("====== Proof verification for Nano Sync initiated ======");

    let start = Instant::now();

    // Verify proof.
    let result = NanoZKP::verify(
        0,
        initial_header_hash,
        initial_pks,
        EPOCH_LENGTH * NUMBER_EPOCHS as u32,
        final_header_hash,
        final_pks,
        proof,
    )
    .unwrap();

    println!("Proof verification finished. It returned {}.", result);

    println!("====== Proof verification for Nano Sync finished ======");
    println!("Total time elapsed: {:?}", start.elapsed());
}
