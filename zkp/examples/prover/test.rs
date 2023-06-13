use std::{io, time::Instant};

use ark_crypto_primitives::snark::{CircuitSpecificSetupSNARK, SNARK};
use ark_groth16::Groth16;
use ark_mnt4_753::MNT4_753;
use log::metadata::LevelFilter;
use nimiq_log::TargetsExt;
use nimiq_zkp_circuits::circuits::mnt6::test::TestCircuit;
use rand::thread_rng;
use tracing_subscriber::{filter::Targets, prelude::*};

fn initialize() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
        .with(
            Targets::new()
                .with_default(LevelFilter::INFO)
                .with_nimiq_targets(LevelFilter::DEBUG)
                .with_target("r1cs", LevelFilter::WARN)
                .with_env(),
        )
        .init();
}

/// Generates the parameters (proving and verifying keys) for the entire zkp circuit.
/// This function will store the parameters in file.
/// Run this example with `cargo run --all-features --release --example setup`.
fn main() {
    initialize();
    let mut rng = thread_rng();
    println!("====== Parameter generation for ZKP Circuit initiated ======");
    let start = Instant::now();
    let circuit = TestCircuit::rand(&mut rng);

    // use the current directory
    let (proving_key, verifying_key) =
        Groth16::<MNT4_753>::setup(circuit.clone(), &mut rng).unwrap();

    println!("====== Parameter generation for ZKP Circuit finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());

    println!("====== Proof generation ======");
    let start = Instant::now();
    let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, &mut rng).unwrap();

    println!("====== Proof generation finished ======");
    println!("Total time elapsed: {:?}", start.elapsed());

    let inputs = vec![];

    let result = Groth16::<MNT4_753>::verify(&verifying_key, &inputs, &proof).unwrap();

    assert!(result)
}
