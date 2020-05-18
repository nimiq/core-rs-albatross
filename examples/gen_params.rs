#![allow(dead_code)]

use std::fs::File;
use std::{error::Error, time::Instant};

use algebra::mnt4_753::MNT4_753;
use algebra::mnt6_753::MNT6_753;
use algebra_core::{test_rng, CanonicalSerialize, ProjectiveCurve};
use groth16::{generate_random_parameters, Parameters, Proof, VerifyingKey};
use rand::RngCore;

use nano_sync::circuits::mnt4::{
    MacroBlockCircuit, MergerCircuit, PKTree1Circuit, PKTree3Circuit, PKTree5Circuit,
};
use nano_sync::circuits::mnt6::{
    MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTree0Circuit, PKTree2Circuit, PKTree4Circuit,
};
use nano_sync::constants::{PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS};
use nano_sync::primitives::MacroBlock;
use nano_sync::utils::{
    bytes_to_bits, gen_rand_g1_mnt4, gen_rand_g1_mnt6, gen_rand_g2_mnt4, gen_rand_g2_mnt6,
};

/// This example generates the parameters (proving and verifying keys) for the entire nano sync
/// program. It does this by generating the parameters for each circuit, "from bottom to top". The
/// order is absolutely necessary because each circuit needs a verifying key from the circuit "below"
/// it. Note that the parameter generation can take longer than one hour, even two on some computers.
fn main() -> Result<(), Box<dyn Error>> {
    println!("====== Parameter generation for Nano Sync initiated ======");
    let start = Instant::now();

    println!("====== PK Tree 5 Circuit ======");
    gen_params_pk_tree_5()?;

    println!("====== PK Tree 4 Circuit ======");
    gen_params_pk_tree_4()?;

    println!("====== PK Tree 3 Circuit ======");
    gen_params_pk_tree_3()?;

    println!("====== PK Tree 2 Circuit ======");
    gen_params_pk_tree_2()?;

    println!("====== PK Tree 1 Circuit ======");
    gen_params_pk_tree_1()?;

    println!("====== PK Tree 0 Circuit ======");
    gen_params_pk_tree_0()?;

    println!("====== Macro Block Circuit ======");
    gen_params_macro_block()?;

    println!("====== Macro Block Wrapper Circuit ======");
    gen_params_macro_block_wrapper()?;

    println!("====== Merger Circuit ======");
    gen_params_merger()?;

    println!("====== Merger Wrapper Circuit ======");
    gen_params_merger_wrapper()?;

    println!("====== Parameter generation for Nano Sync finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());

    Ok(())
}

fn gen_params_pk_tree_5() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

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

    let position = vec![0];

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT4_753> = {
        let c = PKTree5Circuit::new(
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
        "Parameter generation finished. It took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/pk_tree_5.bin")?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/pk_tree_5.bin")?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}

fn gen_params_pk_tree_4() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

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

    let position = vec![0];

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT6_753> = {
        let c = PKTree4Circuit::new(
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
        "Parameter generation finished. It took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/pk_tree_4.bin")?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/pk_tree_4.bin")?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}

fn gen_params_pk_tree_3() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

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

    let position = vec![0];

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT4_753> = {
        let c = PKTree3Circuit::new(
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
        "Parameter generation finished. It took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/pk_tree_3.bin")?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/pk_tree_3.bin")?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}

fn gen_params_pk_tree_2() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

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

    let position = vec![0];

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT6_753> = {
        let c = PKTree2Circuit::new(
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
        "Parameter generation finished. It took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/pk_tree_2.bin")?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/pk_tree_2.bin")?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}

fn gen_params_pk_tree_1() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

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

    let position = vec![0];

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT4_753> = {
        let c = PKTree1Circuit::new(
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
        "Parameter generation finished. It took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/pk_tree_1.bin")?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/pk_tree_1.bin")?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}

fn gen_params_pk_tree_0() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

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

    let position = vec![0];

    // Create parameters for our circuit
    println!("Starting parameter generation.");

    let start = Instant::now();

    let params: Parameters<MNT6_753> = {
        let c = PKTree0Circuit::new(
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
        "Parameter generation finished. It took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/pk_tree_0.bin")?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/pk_tree_0.bin")?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}

fn gen_params_macro_block() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

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

    let initial_block_number = 100;

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
        "Parameter generation finished. It took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/macro_block.bin")?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/macro_block.bin")?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}

fn gen_params_macro_block_wrapper() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

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
            proof,
            initial_state_commitment.to_vec(),
            final_state_commitment.to_vec(),
        );
        generate_random_parameters::<MNT6_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. It took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/macro_block_wrapper.bin")?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/macro_block_wrapper.bin")?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}

fn gen_params_merger() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

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

    let genesis_flag = false;

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
        "Parameter generation finished. It took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/merger.bin")?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/merger.bin")?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}

fn gen_params_merger_wrapper() -> Result<(), Box<dyn Error>> {
    // Initialize rng.
    let rng = &mut test_rng();

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
            proof,
            initial_state_commitment.to_vec(),
            final_state_commitment.to_vec(),
            vk_commitment.to_vec(),
        );
        generate_random_parameters::<MNT6_753, _, _>(c, rng)?
    };

    println!(
        "Parameter generation finished. It took {:?} seconds",
        start.elapsed()
    );

    // Save verifying key to file.
    println!("Storing verifying key");

    let mut file = File::create("verifying_keys/merger_wrapper.bin")?;

    params.vk.serialize(&mut file)?;

    file.sync_all()?;

    // Save proving key to file.
    println!("Storing proving key");

    let mut file = File::create("proving_keys/merger_wrapper.bin")?;

    params.serialize(&mut file)?;

    file.sync_all()?;

    Ok(())
}
