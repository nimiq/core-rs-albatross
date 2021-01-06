#![allow(dead_code)]

use std::fs::{DirBuilder, File};
use std::path::Path;
use std::{error::Error, time::Instant};

use rand::{thread_rng, Rng, RngCore};

use algebra::mnt4_753::{Fr as MNT4Fr, MNT4_753};
use algebra::mnt6_753::{Fr as MNT6Fr, MNT6_753};
use algebra_core::{CanonicalSerialize, PairingEngine, ProjectiveCurve};
use groth16::{generate_random_parameters, Parameters, Proof, VerifyingKey};
use nimiq_nano_sync::circuits::mnt4::{
    MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit as LeafMNT4, PKTreeNodeCircuit as NodeMNT4,
};
use nimiq_nano_sync::circuits::mnt6::{
    MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as NodeMNT6,
};
use nimiq_nano_sync::constants::{PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS};
use nimiq_nano_sync::primitives::MacroBlock;
use nimiq_nano_sync::utils::{
    bytes_to_bits, gen_rand_g1_mnt4, gen_rand_g1_mnt6, gen_rand_g2_mnt4, gen_rand_g2_mnt6,
};
use r1cs_core::ConstraintSynthesizer;

/// This example generates the parameters (proving and verifying keys) for the entire nano sync
/// program. It does this by generating the parameters for each circuit, "from bottom to top". The
/// order is absolutely necessary because each circuit needs a verifying key from the circuit "below"
/// it. Note that the parameter generation can take longer than one hour, even two on some computers.
fn main() -> Result<(), Box<dyn Error>> {
    println!("====== Parameter generation for Nano Sync initiated ======");
    let start = Instant::now();

    println!("====== PK Tree 5 Circuit ======");
    gen_params_pk_tree_leaf("pk_tree_5.bin")?;

    println!("====== PK Tree 4 Circuit ======");
    type Tree4 = LeafMNT4;
    gen_params_pk_tree_node_mnt6::<Tree4>("pk_tree_5.bin", "pk_tree_4.bin")?;

    println!("====== PK Tree 3 Circuit ======");
    type Tree3 = NodeMNT6<LeafMNT4>;
    gen_params_pk_tree_node_mnt4::<Tree3>("pk_tree_4.bin", "pk_tree_3.bin")?;

    println!("====== PK Tree 2 Circuit ======");
    type Tree2 = NodeMNT4<NodeMNT6<LeafMNT4>>;
    gen_params_pk_tree_node_mnt6::<Tree2>("pk_tree_3.bin", "pk_tree_2.bin")?;

    println!("====== PK Tree 1 Circuit ======");
    type Tree1 = NodeMNT6<NodeMNT4<NodeMNT6<LeafMNT4>>>;
    gen_params_pk_tree_node_mnt4::<Tree1>("pk_tree_2.bin", "pk_tree_1.bin")?;

    println!("====== PK Tree 0 Circuit ======");
    type Tree0 = NodeMNT4<NodeMNT6<NodeMNT4<NodeMNT6<LeafMNT4>>>>;
    gen_params_pk_tree_node_mnt6::<Tree0>("pk_tree_1.bin", "pk_tree_0.bin")?;

    println!("====== Macro Block Circuit ======");
    gen_params_macro_block("pk_tree_0.bin", "macro_block.bin")?;

    println!("====== Macro Block Wrapper Circuit ======");
    gen_params_macro_block_wrapper("macro_block.bin", "macro_block_wrapper.bin")?;

    println!("====== Merger Circuit ======");
    gen_params_merger("macro_block_wrapper.bin", "merger.bin")?;

    println!("====== Merger Wrapper Circuit ======");
    gen_params_merger_wrapper("merger.bin", "merger_wrapper.bin")?;

    println!("====== Parameter generation for Nano Sync finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());

    Ok(())
}

fn gen_params_pk_tree_leaf(name: &str) -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Create dummy inputs.
    let pks = vec![gen_rand_g2_mnt6(); VALIDATOR_SLOTS / PK_TREE_BREADTH];

    let pks_nodes = vec![gen_rand_g1_mnt6(); PK_TREE_DEPTH];

    let prepare_agg_pk = gen_rand_g2_mnt6();

    let commit_agg_pk = gen_rand_g2_mnt6();

    let mut pks_commitment = [0u8; 95];
    rng.fill_bytes(&mut pks_commitment);

    let mut prepare_signer_bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut prepare_signer_bitmap);

    let mut prepare_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut prepare_agg_pk_commitment);

    let mut commit_signer_bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut commit_signer_bitmap);

    let mut commit_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut commit_agg_pk_commitment);

    let position: u8 = rng.gen_range(0, 32);

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT4_753> = {
        let c = LeafMNT4::new(
            pks,
            pks_nodes,
            prepare_agg_pk,
            commit_agg_pk,
            pks_commitment.to_vec(),
            prepare_signer_bitmap.to_vec(),
            prepare_agg_pk_commitment.to_vec(),
            commit_signer_bitmap.to_vec(),
            commit_agg_pk_commitment.to_vec(),
            position,
        );
        generate_random_parameters::<MNT4_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(params, name)
}

fn gen_params_pk_tree_node_mnt6<SubCircuit: ConstraintSynthesizer<MNT4Fr>>(
    vk_file: &'static str,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Create dummy inputs.
    let left_proof = Proof {
        a: gen_rand_g1_mnt4().into_affine(),
        b: gen_rand_g2_mnt4().into_affine(),
        c: gen_rand_g1_mnt4().into_affine(),
    };

    let right_proof = Proof {
        a: gen_rand_g1_mnt4().into_affine(),
        b: gen_rand_g2_mnt4().into_affine(),
        c: gen_rand_g1_mnt4().into_affine(),
    };

    let mut pks_commitment = [0u8; 95];
    rng.fill_bytes(&mut pks_commitment);

    let mut prepare_signer_bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut prepare_signer_bitmap);

    let mut left_prepare_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut left_prepare_agg_pk_commitment);

    let mut right_prepare_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut right_prepare_agg_pk_commitment);

    let mut commit_signer_bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut commit_signer_bitmap);

    let mut left_commit_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut left_commit_agg_pk_commitment);

    let mut right_commit_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut right_commit_agg_pk_commitment);

    let position: u8 = rng.gen_range(0, 32);

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT6_753> = {
        let c = NodeMNT6::<SubCircuit>::new(
            vk_file,
            left_proof,
            right_proof,
            pks_commitment.to_vec(),
            prepare_signer_bitmap.to_vec(),
            left_prepare_agg_pk_commitment.to_vec(),
            right_prepare_agg_pk_commitment.to_vec(),
            commit_signer_bitmap.to_vec(),
            left_commit_agg_pk_commitment.to_vec(),
            right_commit_agg_pk_commitment.to_vec(),
            position,
        );
        generate_random_parameters::<MNT6_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(params, name)
}

fn gen_params_pk_tree_node_mnt4<SubCircuit: ConstraintSynthesizer<MNT6Fr>>(
    vk_file: &'static str,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Create dummy inputs.
    let left_proof = Proof {
        a: gen_rand_g1_mnt6().into_affine(),
        b: gen_rand_g2_mnt6().into_affine(),
        c: gen_rand_g1_mnt6().into_affine(),
    };

    let right_proof = Proof {
        a: gen_rand_g1_mnt6().into_affine(),
        b: gen_rand_g2_mnt6().into_affine(),
        c: gen_rand_g1_mnt6().into_affine(),
    };

    let mut prepare_agg_pk_chunks = Vec::new();
    for _ in 0..4 {
        prepare_agg_pk_chunks.push(gen_rand_g2_mnt6());
    }

    let mut commit_agg_pk_chunks = Vec::new();
    for _ in 0..4 {
        commit_agg_pk_chunks.push(gen_rand_g2_mnt6());
    }

    let mut pks_commitment = [0u8; 95];
    rng.fill_bytes(&mut pks_commitment);

    let mut prepare_signer_bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut prepare_signer_bitmap);

    let mut prepare_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut prepare_agg_pk_commitment);

    let mut commit_signer_bitmap = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut commit_signer_bitmap);

    let mut commit_agg_pk_commitment = [0u8; 95];
    rng.fill_bytes(&mut commit_agg_pk_commitment);

    let position: u8 = rng.gen_range(0, 32);

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT4_753> = {
        let c = NodeMNT4::<SubCircuit>::new(
            vk_file,
            left_proof,
            right_proof,
            prepare_agg_pk_chunks,
            commit_agg_pk_chunks,
            pks_commitment.to_vec(),
            prepare_signer_bitmap.to_vec(),
            prepare_agg_pk_commitment.to_vec(),
            commit_signer_bitmap.to_vec(),
            commit_agg_pk_commitment.to_vec(),
            position,
        );
        generate_random_parameters::<MNT4_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(params, name)
}

fn gen_params_macro_block(vk_file: &'static str, name: &str) -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Create dummy inputs.
    let mut prepare_agg_pk_chunks = Vec::new();
    for _ in 0..2 {
        prepare_agg_pk_chunks.push(gen_rand_g2_mnt6());
    }

    let mut commit_agg_pk_chunks = Vec::new();
    for _ in 0..2 {
        commit_agg_pk_chunks.push(gen_rand_g2_mnt6());
    }

    let mut initial_pks_commitment = [0u8; 95];
    rng.fill_bytes(&mut initial_pks_commitment);

    let initial_block_number: u32 = rng.gen_range(0, 1000000);

    let mut final_pks_commitment = [0u8; 95];
    rng.fill_bytes(&mut final_pks_commitment);

    let mut header_hash = [0u8; 32];
    rng.fill_bytes(&mut header_hash);

    let prepare_signature = gen_rand_g1_mnt6();

    let mut bytes = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bytes);
    let prepare_signer_bitmap = bytes_to_bits(&bytes);

    let commit_signature = gen_rand_g1_mnt6();

    let mut bytes = [0u8; VALIDATOR_SLOTS / 8];
    rng.fill_bytes(&mut bytes);
    let commit_signer_bitmap = bytes_to_bits(&bytes);

    let block = MacroBlock {
        header_hash,
        prepare_signature,
        prepare_signer_bitmap,
        commit_signature,
        commit_signer_bitmap,
    };

    let proof = Proof {
        a: gen_rand_g1_mnt6().into_affine(),
        b: gen_rand_g2_mnt6().into_affine(),
        c: gen_rand_g1_mnt6().into_affine(),
    };

    let mut initial_state_commitment = [0u8; 95];
    rng.fill_bytes(&mut initial_state_commitment);

    let mut final_state_commitment = [0u8; 95];
    rng.fill_bytes(&mut final_state_commitment);

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT4_753> = {
        let c = MacroBlockCircuit::new(
            vk_file,
            prepare_agg_pk_chunks,
            commit_agg_pk_chunks,
            initial_pks_commitment.to_vec(),
            initial_block_number,
            final_pks_commitment.to_vec(),
            block,
            proof,
            initial_state_commitment.to_vec(),
            final_state_commitment.to_vec(),
        );
        generate_random_parameters::<MNT4_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(params, name)
}

fn gen_params_macro_block_wrapper(vk_file: &'static str, name: &str) -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Create dummy inputs.
    let proof = Proof {
        a: gen_rand_g1_mnt4().into_affine(),
        b: gen_rand_g2_mnt4().into_affine(),
        c: gen_rand_g1_mnt4().into_affine(),
    };

    let mut initial_state_commitment = [0u8; 95];
    rng.fill_bytes(&mut initial_state_commitment);

    let mut final_state_commitment = [0u8; 95];
    rng.fill_bytes(&mut final_state_commitment);

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT6_753> = {
        let c = MacroBlockWrapperCircuit::new(
            vk_file,
            proof,
            initial_state_commitment.to_vec(),
            final_state_commitment.to_vec(),
        );
        generate_random_parameters::<MNT6_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(params, name)
}

fn gen_params_merger(vk_file: &'static str, name: &str) -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Create dummy inputs.
    let proof_merger_wrapper = Proof {
        a: gen_rand_g1_mnt6().into_affine(),
        b: gen_rand_g2_mnt6().into_affine(),
        c: gen_rand_g1_mnt6().into_affine(),
    };

    let proof_macro_block_wrapper = Proof {
        a: gen_rand_g1_mnt6().into_affine(),
        b: gen_rand_g2_mnt6().into_affine(),
        c: gen_rand_g1_mnt6().into_affine(),
    };

    let vk_merger_wrapper = VerifyingKey {
        alpha_g1: gen_rand_g1_mnt6().into_affine(),
        beta_g2: gen_rand_g2_mnt6().into_affine(),
        gamma_g2: gen_rand_g2_mnt6().into_affine(),
        delta_g2: gen_rand_g2_mnt6().into_affine(),
        gamma_abc_g1: vec![gen_rand_g1_mnt6().into_affine(); 7],
    };

    let mut intermediate_state_commitment = [0u8; 95];
    rng.fill_bytes(&mut intermediate_state_commitment);

    let genesis_flag: bool = rng.gen();

    let mut initial_state_commitment = [0u8; 95];
    rng.fill_bytes(&mut initial_state_commitment);

    let mut final_state_commitment = [0u8; 95];
    rng.fill_bytes(&mut final_state_commitment);

    let mut vk_commitment = [0u8; 95];
    rng.fill_bytes(&mut vk_commitment);

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT4_753> = {
        let c = MergerCircuit::new(
            vk_file,
            proof_merger_wrapper,
            proof_macro_block_wrapper,
            vk_merger_wrapper,
            intermediate_state_commitment.to_vec(),
            genesis_flag,
            initial_state_commitment.to_vec(),
            final_state_commitment.to_vec(),
            vk_commitment.to_vec(),
        );
        generate_random_parameters::<MNT4_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(params, name)
}

fn gen_params_merger_wrapper(vk_file: &'static str, name: &str) -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Create dummy inputs.
    let proof = Proof {
        a: gen_rand_g1_mnt4().into_affine(),
        b: gen_rand_g2_mnt4().into_affine(),
        c: gen_rand_g1_mnt4().into_affine(),
    };

    let mut initial_state_commitment = [0u8; 95];
    rng.fill_bytes(&mut initial_state_commitment);

    let mut final_state_commitment = [0u8; 95];
    rng.fill_bytes(&mut final_state_commitment);

    let mut vk_commitment = [0u8; 95];
    rng.fill_bytes(&mut vk_commitment);

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT6_753> = {
        let c = MergerWrapperCircuit::new(
            vk_file,
            proof,
            initial_state_commitment.to_vec(),
            final_state_commitment.to_vec(),
            vk_commitment.to_vec(),
        );
        generate_random_parameters::<MNT6_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save keys to file.
    to_file(params, name)
}

fn to_file<T: PairingEngine>(params: Parameters<T>, name: &str) -> Result<(), Box<dyn Error>> {
    // Save verifying key to file.
    println!("Storing verifying key.");

    if !Path::new("verifying_keys/").is_dir() {
        DirBuilder::new().create("verifying_keys/").unwrap();
    }

    let mut file = File::create(format!("verifying_keys/{}", name))?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key.");

    if !Path::new("proving_keys/").is_dir() {
        DirBuilder::new().create("proving_keys/").unwrap();
    }

    let mut file = File::create(format!("proving_keys/{}", name))?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}
