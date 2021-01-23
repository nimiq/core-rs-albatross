use std::fs::{DirBuilder, File};
use std::path::Path;
use std::time::Instant;

use ark_crypto_primitives::CircuitSpecificSetupSNARK;
use ark_ec::{PairingEngine, ProjectiveCurve};
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_mnt4_753::{Fr as MNT4Fr, G1Projective as G1MNT4, G2Projective as G2MNT4, MNT4_753};
use ark_mnt6_753::{Fr as MNT6Fr, G1Projective as G1MNT6, G2Projective as G2MNT6, MNT6_753};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::UniformRand;
use rand::{thread_rng, RngCore};

use nimiq_nano_sync::circuits::mnt4::{
    MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit as LeafMNT4, PKTreeNodeCircuit as NodeMNT4,
};
use nimiq_nano_sync::circuits::mnt6::{
    MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as NodeMNT6,
};
use nimiq_nano_sync::constants::{PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS};
use nimiq_nano_sync::primitives::MacroBlock;
use nimiq_nano_sync::utils::bytes_to_bits;

/// This example generates the parameters (proving and verifying keys) for the entire nano sync
/// program. It does this by generating the parameters for each circuit, "from bottom to top". The
/// order is absolutely necessary because each circuit needs a verifying key from the circuit "below"
/// it. Note that the parameter generation can take longer than one hour, even two on some computers.
fn main() {
    println!("====== Parameter generation for Nano Sync initiated ======");
    let start = Instant::now();

    println!("====== PK Tree 5 Circuit ======");
    setup_pk_tree_leaf("pk_tree_5");

    println!("====== PK Tree 4 Circuit ======");
    setup_pk_tree_node_mnt6("pk_tree_5", "pk_tree_4");

    println!("====== PK Tree 3 Circuit ======");
    setup_pk_tree_node_mnt4("pk_tree_4", "pk_tree_3");

    println!("====== PK Tree 2 Circuit ======");
    setup_pk_tree_node_mnt6("pk_tree_3", "pk_tree_2");

    println!("====== PK Tree 1 Circuit ======");
    setup_pk_tree_node_mnt4("pk_tree_2", "pk_tree_1");

    println!("====== PK Tree 0 Circuit ======");
    setup_pk_tree_node_mnt6("pk_tree_1", "pk_tree_0");

    println!("====== Macro Block Circuit ======");
    setup_macro_block("pk_tree_0", "macro_block");

    println!("====== Macro Block Wrapper Circuit ======");
    setup_macro_block_wrapper("macro_block", "macro_block_wrapper");

    println!("====== Merger Circuit ======");
    setup_merger("macro_block_wrapper", "merger");

    println!("====== Merger Wrapper Circuit ======");
    setup_merger_wrapper("merger", "merger_wrapper");

    println!("====== Parameter generation for Nano Sync finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());
}

fn setup_pk_tree_leaf(name: &str) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Create dummy inputs.
    let position = 0;

    let pks = vec![G2MNT6::rand(rng); VALIDATOR_SLOTS / PK_TREE_BREADTH];

    let pk_tree_nodes = vec![G1MNT6::rand(rng); PK_TREE_DEPTH];

    let pk_tree_root = vec![MNT4Fr::rand(rng); 2];

    let agg_pk_commitment = vec![MNT4Fr::rand(rng); 2];

    let signer_bitmap = MNT4Fr::rand(rng);

    let path = MNT4Fr::rand(rng);

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let circuit = LeafMNT4::new(
        position,
        pks,
        pk_tree_nodes,
        pk_tree_root,
        agg_pk_commitment,
        signer_bitmap,
        path,
    );

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng).unwrap();

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(pk, vk, name)
}

fn setup_pk_tree_node_mnt6(vk_file: &'static str, name: &str) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/{}.bin", vk_file)).unwrap();

    let vk_child = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Create dummy inputs.
    let left_proof = Proof {
        a: G1MNT4::rand(rng).into_affine(),
        b: G2MNT4::rand(rng).into_affine(),
        c: G1MNT4::rand(rng).into_affine(),
    };

    let right_proof = Proof {
        a: G1MNT4::rand(rng).into_affine(),
        b: G2MNT4::rand(rng).into_affine(),
        c: G1MNT4::rand(rng).into_affine(),
    };

    let pk_tree_root = vec![MNT6Fr::rand(rng); 2];

    let left_agg_pk_commitment = vec![MNT6Fr::rand(rng); 2];

    let right_agg_pk_commitment = vec![MNT6Fr::rand(rng); 2];

    let signer_bitmap = MNT6Fr::rand(rng);

    let path = MNT6Fr::rand(rng);

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let circuit = NodeMNT6::new(
        vk_child,
        left_proof,
        right_proof,
        pk_tree_root,
        left_agg_pk_commitment,
        right_agg_pk_commitment,
        signer_bitmap,
        path,
    );

    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng).unwrap();

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(pk, vk, name)
}

fn setup_pk_tree_node_mnt4(vk_file: &'static str, name: &str) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/{}.bin", vk_file)).unwrap();

    let vk_child = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Create dummy inputs.
    let left_proof = Proof {
        a: G1MNT6::rand(rng).into_affine(),
        b: G2MNT6::rand(rng).into_affine(),
        c: G1MNT6::rand(rng).into_affine(),
    };

    let right_proof = Proof {
        a: G1MNT6::rand(rng).into_affine(),
        b: G2MNT6::rand(rng).into_affine(),
        c: G1MNT6::rand(rng).into_affine(),
    };

    let agg_pk_chunks = vec![G2MNT6::rand(rng); 4];

    let pk_tree_root = vec![MNT4Fr::rand(rng); 2];

    let agg_pk_commitment = vec![MNT4Fr::rand(rng); 2];

    let signer_bitmap = MNT4Fr::rand(rng);

    let path = MNT4Fr::rand(rng);

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let circuit = NodeMNT4::new(
        vk_child,
        left_proof,
        right_proof,
        agg_pk_chunks,
        pk_tree_root,
        agg_pk_commitment,
        signer_bitmap,
        path,
    );

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng).unwrap();

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(pk, vk, name)
}

fn setup_macro_block(vk_file: &'static str, name: &str) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/{}.bin", vk_file)).unwrap();

    let vk_pk_tree = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Create dummy inputs.
    let agg_pk_chunks = vec![G2MNT6::rand(rng); 2];

    let proof = Proof {
        a: G1MNT6::rand(rng).into_affine(),
        b: G2MNT6::rand(rng).into_affine(),
        c: G1MNT6::rand(rng).into_affine(),
    };

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let initial_pk_tree_root = bytes_to_bits(&bytes);

    let initial_block_number = u32::rand(rng);

    let initial_round_number = u32::rand(rng);

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let final_pk_tree_root = bytes_to_bits(&bytes);

    let initial_state_commitment = vec![MNT4Fr::rand(rng); 2];

    let final_state_commitment = vec![MNT4Fr::rand(rng); 2];

    let mut header_hash = [0u8; 32];
    rng.fill_bytes(&mut header_hash);

    let signature = G1MNT6::rand(rng);

    let mut bytes = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bytes);
    let signer_bitmap = bytes_to_bits(&bytes);

    let block = MacroBlock {
        header_hash,
        signature,
        signer_bitmap,
    };

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let circuit = MacroBlockCircuit::new(
        vk_pk_tree,
        agg_pk_chunks,
        proof,
        initial_pk_tree_root,
        initial_block_number,
        initial_round_number,
        final_pk_tree_root,
        block,
        initial_state_commitment,
        final_state_commitment,
    );

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng).unwrap();

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(pk, vk, name)
}

fn setup_macro_block_wrapper(vk_file: &'static str, name: &str) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/{}.bin", vk_file)).unwrap();

    let vk_macro_block = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Create dummy inputs.
    let proof = Proof {
        a: G1MNT4::rand(rng).into_affine(),
        b: G2MNT4::rand(rng).into_affine(),
        c: G1MNT4::rand(rng).into_affine(),
    };

    let initial_state_commitment = vec![MNT6Fr::rand(rng); 2];

    let final_state_commitment = vec![MNT6Fr::rand(rng); 2];

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let circuit = MacroBlockWrapperCircuit::new(
        vk_macro_block,
        proof,
        initial_state_commitment,
        final_state_commitment,
    );

    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng).unwrap();

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(pk, vk, name)
}

fn setup_merger(vk_file: &'static str, name: &str) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/{}.bin", vk_file)).unwrap();

    let vk_macro_block_wrapper = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Create dummy inputs.
    let proof_merger_wrapper = Proof {
        a: G1MNT6::rand(rng).into_affine(),
        b: G2MNT6::rand(rng).into_affine(),
        c: G1MNT6::rand(rng).into_affine(),
    };

    let proof_macro_block_wrapper = Proof {
        a: G1MNT6::rand(rng).into_affine(),
        b: G2MNT6::rand(rng).into_affine(),
        c: G1MNT6::rand(rng).into_affine(),
    };

    let vk_merger_wrapper = VerifyingKey {
        alpha_g1: G1MNT6::rand(rng).into_affine(),
        beta_g2: G2MNT6::rand(rng).into_affine(),
        gamma_g2: G2MNT6::rand(rng).into_affine(),
        delta_g2: G2MNT6::rand(rng).into_affine(),
        gamma_abc_g1: vec![G1MNT6::rand(rng).into_affine(); 7],
    };

    let mut bytes = [0u8; 95];
    rng.fill_bytes(&mut bytes);
    let intermediate_state_commitment = bytes_to_bits(&bytes);

    let genesis_flag = bool::rand(rng);

    let initial_state_commitment = vec![MNT4Fr::rand(rng); 2];

    let final_state_commitment = vec![MNT4Fr::rand(rng); 2];

    let vk_commitment = vec![MNT4Fr::rand(rng); 2];

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let circuit = MergerCircuit::new(
        vk_macro_block_wrapper,
        proof_merger_wrapper,
        proof_macro_block_wrapper,
        vk_merger_wrapper,
        intermediate_state_commitment,
        genesis_flag,
        initial_state_commitment,
        final_state_commitment,
        vk_commitment,
    );

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng).unwrap();

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(pk, vk, name)
}

fn setup_merger_wrapper(vk_file: &'static str, name: &str) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/{}.bin", vk_file)).unwrap();

    let vk_merger = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Create dummy inputs.
    let proof = Proof {
        a: G1MNT4::rand(rng).into_affine(),
        b: G2MNT4::rand(rng).into_affine(),
        c: G1MNT4::rand(rng).into_affine(),
    };

    let initial_state_commitment = vec![MNT6Fr::rand(rng); 2];

    let final_state_commitment = vec![MNT6Fr::rand(rng); 2];

    let vk_commitment = vec![MNT6Fr::rand(rng); 2];

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let circuit = MergerWrapperCircuit::new(
        vk_merger,
        proof,
        initial_state_commitment,
        final_state_commitment,
        vk_commitment,
    );

    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng).unwrap();

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(pk, vk, name)
}

fn to_file<T: PairingEngine>(pk: ProvingKey<T>, vk: VerifyingKey<T>, name: &str) {
    // Save proving key to file.
    println!("Storing proving key.");

    if !Path::new("proving_keys/").is_dir() {
        DirBuilder::new().create("proving_keys/").unwrap();
    }

    let mut file = File::create(format!("proving_keys/{}.bin", name)).unwrap();

    pk.serialize_unchecked(&mut file).unwrap();

    file.sync_all().unwrap();

    // Save verifying key to file.
    println!("Storing verifying key.");

    if !Path::new("verifying_keys/").is_dir() {
        DirBuilder::new().create("verifying_keys/").unwrap();
    }

    let mut file = File::create(format!("verifying_keys/{}.bin", name)).unwrap();

    vk.serialize_unchecked(&mut file).unwrap();

    file.sync_all().unwrap();
}
