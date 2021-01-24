use std::fs;
use std::fs::{DirBuilder, File};
use std::path::Path;
use std::time::Instant;

use ark_crypto_primitives::SNARK;
use ark_ec::{PairingEngine, ProjectiveCurve};
use ark_ff::Zero;
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_mnt4_753::MNT4_753;
use ark_mnt6_753::{Fr as MNT6Fr, G1Projective as G1MNT6, G2Projective as G2MNT6, MNT6_753};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::ops::MulAssign;
use ark_std::UniformRand;
use rand::prelude::SliceRandom;
use rand::{thread_rng, RngCore};

use nimiq_bls::pedersen::{pedersen_generators, pedersen_hash};
use nimiq_nano_sync::circuits::mnt4::{
    MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit as LeafMNT4, PKTreeNodeCircuit as NodeMNT4,
};
use nimiq_nano_sync::circuits::mnt6::{
    MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as NodeMNT6,
};
use nimiq_nano_sync::constants::{
    EPOCH_LENGTH, MIN_SIGNERS, PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS,
};
use nimiq_nano_sync::primitives::{
    merkle_tree_prove, pk_tree_construct, serialize_g1_mnt6, serialize_g2_mnt6, state_commitment,
    vk_commitment, MacroBlock,
};
use nimiq_nano_sync::utils::{byte_to_le_bits, bytes_to_bits, prepare_inputs};

/// This example generates a proof for a genesis block, it uses the entire nano sync program. It
/// also stores the random inputs on which the proof was created so that the proof can be verified
/// later. Note that the proof generation can easily take longer than 12 hours.
fn main() {
    println!("====== Proof generation for Nano Sync initiated ======");
    let start = Instant::now();

    println!("====== Generating random inputs ======");
    let (
        initial_pks,
        pk_tree_proofs,
        initial_pk_tree_root,
        final_pks,
        final_pk_tree_root,
        block_number,
        round_number,
        block,
    ) = generate_inputs();

    println!("====== Generating proofs ======");
    // Start generating proofs for PKTree level 5.
    for i in 0..32 {
        println!("PKTree 5 circuit - number {}:", i);
        prove_pk_tree_leaf(
            "pk_tree_5",
            i,
            &initial_pks,
            &pk_tree_proofs[i],
            &initial_pk_tree_root,
            &block.signer_bitmap,
        );
    }

    // Start generating proofs for PKTree level 4.
    for i in 0..16 {
        println!("PKTree 4 circuit - number {}:", i);
        prove_pk_tree_node_mnt6(
            "pk_tree_4",
            i,
            4,
            "pk_tree_5",
            &initial_pks,
            &initial_pk_tree_root,
            &block.signer_bitmap,
        );
    }

    // Start generating proofs for PKTree level 3.
    for i in 0..8 {
        println!("PKTree 3 circuit - number {}:", i);
        prove_pk_tree_node_mnt4(
            "pk_tree_3",
            i,
            3,
            "pk_tree_4",
            &initial_pks,
            &initial_pk_tree_root,
            &block.signer_bitmap,
        );
    }

    // Start generating proofs for PKTree level 2.
    for i in 0..4 {
        println!("PKTree 2 circuit - number {}:", i);
        prove_pk_tree_node_mnt6(
            "pk_tree_2",
            i,
            2,
            "pk_tree_3",
            &initial_pks,
            &initial_pk_tree_root,
            &block.signer_bitmap,
        );
    }

    // Start generating proofs for PKTree level 1.
    for i in 0..2 {
        println!("PKTree 1 circuit - number {}:", i);
        prove_pk_tree_node_mnt4(
            "pk_tree_1",
            i,
            1,
            "pk_tree_2",
            &initial_pks,
            &initial_pk_tree_root,
            &block.signer_bitmap,
        );
    }

    // Start generating proof for PKTree level 0.
    println!("PKTree 0 circuit");
    prove_pk_tree_node_mnt6(
        "pk_tree_0",
        0,
        0,
        "pk_tree_1",
        &initial_pks,
        &initial_pk_tree_root,
        &block.signer_bitmap,
    );

    // Start generating proof for Macro Block.
    println!("Macro Block circuit");
    prove_macro_block(
        &initial_pks,
        &initial_pk_tree_root,
        &final_pks,
        &final_pk_tree_root,
        block_number,
        round_number,
        block,
    );

    // Start generating proof for Macro Block Wrapper.
    println!("Macro Block Wrapper circuit");
    prove_macro_block_wrapper(&initial_pks, &final_pks, block_number);

    // Start generating proof for Merger.
    println!("Merger circuit");
    prove_merger(&initial_pks, &final_pks, block_number);

    // Start generating proof for Merger Wrapper.
    println!("Merger Wrapper circuit");
    prove_merger_wrapper(&initial_pks, &final_pks, block_number);

    println!("====== Cleaning up ======");
    // Save state commitments to file.
    state_comms_to_file(
        state_commitment(block_number, initial_pks.to_vec()),
        state_commitment(block_number + EPOCH_LENGTH, final_pks.to_vec()),
    );

    // Remove useless proofs.
    for i in 0..32 {
        fs::remove_file(format!("proofs/pk_tree_5_{}.bin", i)).unwrap();
    }

    for i in 0..16 {
        fs::remove_file(format!("proofs/pk_tree_4_{}.bin", i)).unwrap();
    }

    for i in 0..8 {
        fs::remove_file(format!("proofs/pk_tree_3_{}.bin", i)).unwrap();
    }

    for i in 0..4 {
        fs::remove_file(format!("proofs/pk_tree_2_{}.bin", i)).unwrap();
    }

    for i in 0..2 {
        fs::remove_file(format!("proofs/pk_tree_1_{}.bin", i)).unwrap();
    }

    fs::remove_file("proofs/pk_tree_0_0.bin").unwrap();

    fs::remove_file("proofs/macro_block.bin").unwrap();

    fs::remove_file("proofs/macro_block_wrapper.bin").unwrap();

    fs::remove_file("proofs/merger.bin").unwrap();

    // Rename useful proof.
    fs::rename("proofs/merger_wrapper.bin", "proofs/proof_epoch_0.bin").unwrap();

    println!("====== Proof generation for Nano Sync finished ======");
    println!("Total time elapsed: {:?}", start.elapsed());
}

fn generate_inputs() -> (
    Vec<G2MNT6>,
    Vec<Vec<G1MNT6>>,
    Vec<u8>,
    Vec<G2MNT6>,
    Vec<u8>,
    u32,
    u32,
    MacroBlock,
) {
    let start = Instant::now();

    // Initialize rng.
    let rng = &mut thread_rng();

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

    // Serialize the public keys into bits and chunk them into the number of leaves.
    let mut bytes = Vec::new();

    for i in 0..initial_pks.len() {
        bytes.extend_from_slice(&serialize_g2_mnt6(initial_pks[i].clone()));
    }

    let bits = bytes_to_bits(&bytes);

    let mut pks_bits = Vec::new();

    for i in 0..PK_TREE_BREADTH {
        pks_bits.push(
            bits[i * bits.len() / PK_TREE_BREADTH..(i + 1) * bits.len() / PK_TREE_BREADTH].to_vec(),
        );
    }

    // Calculate the Merkle proofs for each leaf.
    let mut pk_tree_proofs = vec![];

    for i in 0..PK_TREE_BREADTH {
        let mut path = byte_to_le_bits(i as u8);

        path.truncate(PK_TREE_DEPTH);

        pk_tree_proofs.push(merkle_tree_prove(pks_bits.clone(), path));
    }

    // Calculate initial public key tree root.
    let initial_pk_tree_root = pk_tree_construct(initial_pks.clone());

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

    // Calculate final public key tree root.
    let final_pk_tree_root = pk_tree_construct(final_pks.clone());

    // Create a random signer bitmap.
    let mut signer_bitmap = vec![true; MIN_SIGNERS];

    signer_bitmap.append(&mut vec![false; VALIDATOR_SLOTS - MIN_SIGNERS]);

    signer_bitmap.shuffle(rng);

    // Create a macro block
    let block_number = 0;

    let round_number = 0;

    let mut header_hash = [0u8; 32];
    rng.fill_bytes(&mut header_hash);

    let mut block = MacroBlock::without_signatures(header_hash);

    for i in 0..VALIDATOR_SLOTS {
        if signer_bitmap[i] {
            block.sign(
                initial_sks[i].clone(),
                i,
                block_number,
                round_number,
                final_pk_tree_root.clone(),
            );
        }
    }

    println!("Input generation finished. It took {:?}.", start.elapsed());

    (
        initial_pks,
        pk_tree_proofs,
        initial_pk_tree_root,
        final_pks,
        final_pk_tree_root,
        block_number,
        round_number,
        block,
    )
}

fn prove_pk_tree_leaf(
    name: &str,
    position: usize,
    pks: &[G2MNT6],
    pk_tree_nodes: &Vec<G1MNT6>,
    pk_tree_root: &Vec<u8>,
    signer_bitmap: &Vec<bool>,
) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the proving key from file.
    let mut file = File::open(format!("proving_keys/{}.bin", name)).unwrap();

    let proving_key = ProvingKey::deserialize_unchecked(&mut file).unwrap();

    // Calculate the aggregate public key commitment.
    let mut agg_pk = G2MNT6::zero();

    for i in position * VALIDATOR_SLOTS / PK_TREE_BREADTH
        ..(position + 1) * VALIDATOR_SLOTS / PK_TREE_BREADTH
    {
        if signer_bitmap[i] {
            agg_pk += pks[i];
        }
    }

    let agg_pk_bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk));

    let hash = pedersen_hash(agg_pk_bits, pedersen_generators(5));

    let agg_pk_comm = bytes_to_bits(&serialize_g1_mnt6(hash));

    // Calculate inputs.
    let pk_tree_root = prepare_inputs(bytes_to_bits(pk_tree_root));

    let agg_pk_commitment = prepare_inputs(agg_pk_comm);

    let signer_bitmap = prepare_inputs(signer_bitmap.clone()).pop().unwrap();

    let path = prepare_inputs(byte_to_le_bits(position as u8))
        .pop()
        .unwrap();

    // Create the proof.
    println!("Starting proof generation.");

    let start = Instant::now();

    let circuit = LeafMNT4::new(
        position,
        pks[position * VALIDATOR_SLOTS / PK_TREE_BREADTH
            ..(position + 1) * VALIDATOR_SLOTS / PK_TREE_BREADTH]
            .to_vec(),
        pk_tree_nodes.to_vec(),
        pk_tree_root,
        agg_pk_commitment,
        signer_bitmap,
        path,
    );

    let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng).unwrap();

    println!("Proof generation finished. It took {:?}.", start.elapsed());

    // Save proof to file.
    proof_to_file(proof, name, Some(position))
}

fn prove_pk_tree_node_mnt6(
    name: &str,
    position: usize,
    level: usize,
    vk_file: &str,
    pks: &[G2MNT6],
    pk_tree_root: &Vec<u8>,
    signer_bitmap: &Vec<bool>,
) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the proving key from file.
    let mut file = File::open(format!("proving_keys/{}.bin", name)).unwrap();

    let proving_key = ProvingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/{}.bin", vk_file)).unwrap();

    let vk_child = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the left proof from file.
    let left_position = 2 * position;

    let mut file = File::open(format!("proofs/{}_{}.bin", vk_file, left_position)).unwrap();

    let left_proof = Proof::deserialize_unchecked(&mut file).unwrap();

    // Load the right proof from file.
    let right_position = 2 * position + 1;

    let mut file = File::open(format!("proofs/{}_{}.bin", vk_file, right_position)).unwrap();

    let right_proof = Proof::deserialize_unchecked(&mut file).unwrap();

    // Calculate the left aggregate public key commitment.
    let mut agg_pk = G2MNT6::zero();

    for i in left_position * VALIDATOR_SLOTS / 2_usize.pow((level + 1) as u32)
        ..(left_position + 1) * VALIDATOR_SLOTS / 2_usize.pow((level + 1) as u32)
    {
        if signer_bitmap[i] {
            agg_pk += pks[i];
        }
    }

    let agg_pk_bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk));

    let hash = pedersen_hash(agg_pk_bits, pedersen_generators(5));

    let left_agg_pk_comm = bytes_to_bits(&serialize_g1_mnt6(hash));

    // Calculate the right aggregate public key commitment.
    let mut agg_pk = G2MNT6::zero();

    for i in right_position * VALIDATOR_SLOTS / 2_usize.pow((level + 1) as u32)
        ..(right_position + 1) * VALIDATOR_SLOTS / 2_usize.pow((level + 1) as u32)
    {
        if signer_bitmap[i] {
            agg_pk += pks[i];
        }
    }

    let agg_pk_bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk));

    let hash = pedersen_hash(agg_pk_bits, pedersen_generators(5));

    let right_agg_pk_comm = bytes_to_bits(&serialize_g1_mnt6(hash));

    // Calculate inputs.
    let pk_tree_root = prepare_inputs(bytes_to_bits(pk_tree_root));

    let left_agg_pk_commitment = prepare_inputs(left_agg_pk_comm);

    let right_agg_pk_commitment = prepare_inputs(right_agg_pk_comm);

    let signer_bitmap = prepare_inputs(signer_bitmap.clone()).pop().unwrap();

    let path = prepare_inputs(byte_to_le_bits(position as u8))
        .pop()
        .unwrap();

    // Create the proof.
    println!("Starting proof generation.");

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

    let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng).unwrap();

    println!("Proof generation finished. It took {:?}.", start.elapsed());

    // Save proof to file.
    proof_to_file(proof, name, Some(position))
}

fn prove_pk_tree_node_mnt4(
    name: &str,
    position: usize,
    level: usize,
    vk_file: &str,
    pks: &[G2MNT6],
    pk_tree_root: &Vec<u8>,
    signer_bitmap: &Vec<bool>,
) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the proving key from file.
    let mut file = File::open(format!("proving_keys/{}.bin", name)).unwrap();

    let proving_key = ProvingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/{}.bin", vk_file)).unwrap();

    let vk_child = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the left proof from file.
    let left_position = 2 * position;

    let mut file = File::open(format!("proofs/{}_{}.bin", vk_file, left_position)).unwrap();

    let left_proof = Proof::deserialize_unchecked(&mut file).unwrap();

    // Load the right proof from file.
    let right_position = 2 * position + 1;

    let mut file = File::open(format!("proofs/{}_{}.bin", vk_file, right_position)).unwrap();

    let right_proof = Proof::deserialize_unchecked(&mut file).unwrap();

    // Calculate the aggregate public key chunks.
    let mut agg_pk_chunks = vec![];

    for i in position * 4..(position + 1) * 4 {
        let mut agg_pk = G2MNT6::zero();

        for j in i * VALIDATOR_SLOTS / 2_usize.pow((level + 2) as u32)
            ..(i + 1) * VALIDATOR_SLOTS / 2_usize.pow((level + 2) as u32)
        {
            if signer_bitmap[j] {
                agg_pk += pks[j];
            }
        }

        agg_pk_chunks.push(agg_pk);
    }

    // Calculate the aggregate public key commitment.
    let mut agg_pk = G2MNT6::zero();

    for chunk in &agg_pk_chunks {
        agg_pk += chunk;
    }

    let agg_pk_bits = bytes_to_bits(&serialize_g2_mnt6(agg_pk));

    let hash = pedersen_hash(agg_pk_bits, pedersen_generators(5));

    let agg_pk_comm = bytes_to_bits(&serialize_g1_mnt6(hash));

    // Calculate inputs.
    let pk_tree_root = prepare_inputs(bytes_to_bits(pk_tree_root));

    let agg_pk_commitment = prepare_inputs(agg_pk_comm);

    let signer_bitmap = prepare_inputs(signer_bitmap.clone()).pop().unwrap();

    let path = prepare_inputs(byte_to_le_bits(position as u8))
        .pop()
        .unwrap();

    // Create the proof.
    println!("Starting proof generation.");

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

    let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng).unwrap();

    println!("Proof generation finished. It took {:?}.", start.elapsed());

    // Save proof to file.
    proof_to_file(proof, name, Some(position))
}

fn prove_macro_block(
    initial_pks: &[G2MNT6],
    initial_pk_tree_root: &Vec<u8>,
    final_pks: &[G2MNT6],
    final_pk_tree_root: &Vec<u8>,
    block_number: u32,
    round_number: u32,
    block: MacroBlock,
) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the proving key from file.
    let mut file = File::open(format!("proving_keys/macro_block.bin")).unwrap();

    let proving_key = ProvingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/pk_tree_0.bin")).unwrap();

    let vk_pk_tree = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the proof from file.
    let mut file = File::open(format!("proofs/pk_tree_0_0.bin")).unwrap();

    let proof = Proof::deserialize_unchecked(&mut file).unwrap();

    // Calculate the aggregate public key chunks.
    let mut agg_pk_chunks = vec![];

    for i in 0..2 {
        let mut agg_pk = G2MNT6::zero();

        for j in i * VALIDATOR_SLOTS / 2..(i + 1) * VALIDATOR_SLOTS / 2 {
            if block.signer_bitmap[j] {
                agg_pk += initial_pks[j];
            }
        }

        agg_pk_chunks.push(agg_pk);
    }

    // Calculate the inputs.
    let initial_state_commitment = prepare_inputs(bytes_to_bits(&state_commitment(
        block_number,
        initial_pks.to_vec(),
    )));

    let final_state_commitment = prepare_inputs(bytes_to_bits(&state_commitment(
        block_number + EPOCH_LENGTH,
        final_pks.to_vec(),
    )));

    // Create the proof.
    println!("Starting proof generation.");

    let start = Instant::now();

    let circuit = MacroBlockCircuit::new(
        vk_pk_tree,
        agg_pk_chunks,
        proof,
        bytes_to_bits(initial_pk_tree_root),
        block_number,
        round_number,
        bytes_to_bits(final_pk_tree_root),
        block,
        initial_state_commitment,
        final_state_commitment,
    );

    let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng).unwrap();

    println!("Proof generation finished. It took {:?}.", start.elapsed());

    // Save proof to file.
    proof_to_file(proof, "macro_block", None)
}

fn prove_macro_block_wrapper(initial_pks: &[G2MNT6], final_pks: &[G2MNT6], block_number: u32) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the proving key from file.
    let mut file = File::open(format!("proving_keys/macro_block_wrapper.bin")).unwrap();

    let proving_key = ProvingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/macro_block.bin")).unwrap();

    let vk_macro_block = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the proof from file.
    let mut file = File::open(format!("proofs/macro_block.bin")).unwrap();

    let proof = Proof::deserialize_unchecked(&mut file).unwrap();

    // Calculate the inputs.
    let initial_state_commitment = prepare_inputs(bytes_to_bits(&state_commitment(
        block_number,
        initial_pks.to_vec(),
    )));

    let final_state_commitment = prepare_inputs(bytes_to_bits(&state_commitment(
        block_number + EPOCH_LENGTH,
        final_pks.to_vec(),
    )));

    // Create the proof.
    println!("Starting proof generation.");

    let start = Instant::now();

    let circuit = MacroBlockWrapperCircuit::new(
        vk_macro_block,
        proof,
        initial_state_commitment,
        final_state_commitment,
    );

    let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng).unwrap();

    println!("Proof generation finished. It took {:?}.", start.elapsed());

    // Save proof to file.
    proof_to_file(proof, "macro_block_wrapper", None)
}

fn prove_merger(initial_pks: &[G2MNT6], final_pks: &[G2MNT6], block_number: u32) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the proving key from file.
    let mut file = File::open(format!("proving_keys/merger.bin")).unwrap();

    let proving_key = ProvingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the verifying key for Macro Block Wrapper from file.
    let mut file = File::open(format!("verifying_keys/macro_block_wrapper.bin")).unwrap();

    let vk_macro_block_wrapper = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the proof for Macro Block Wrapper from file.
    let mut file = File::open(format!("proofs/macro_block_wrapper.bin")).unwrap();

    let proof_macro_block_wrapper = Proof::deserialize_unchecked(&mut file).unwrap();

    // Load the verifying key for Merger Wrapper from file.
    let mut file = File::open(format!("verifying_keys/merger_wrapper.bin")).unwrap();

    let vk_merger_wrapper = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Since we are generating a proof for the genesis block, we don't have a previous Merger Wrapper
    // proof. So we'll just create a random one.
    let proof_merger_wrapper = Proof {
        a: G1MNT6::rand(rng).into_affine(),
        b: G2MNT6::rand(rng).into_affine(),
        c: G1MNT6::rand(rng).into_affine(),
    };

    // Calculate the inputs.
    let initial_state_comm_bits =
        bytes_to_bits(&state_commitment(block_number, initial_pks.to_vec()));

    let initial_state_commitment = prepare_inputs(initial_state_comm_bits.clone());

    let final_state_commitment = prepare_inputs(bytes_to_bits(&state_commitment(
        block_number + EPOCH_LENGTH,
        final_pks.to_vec(),
    )));

    let vk_commitment = prepare_inputs(bytes_to_bits(&vk_commitment(vk_merger_wrapper.clone())));

    // Create the proof.
    println!("Starting proof generation.");

    let start = Instant::now();

    let circuit = MergerCircuit::new(
        vk_macro_block_wrapper,
        proof_merger_wrapper,
        proof_macro_block_wrapper,
        vk_merger_wrapper,
        initial_state_comm_bits,
        true,
        initial_state_commitment,
        final_state_commitment,
        vk_commitment,
    );

    let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng).unwrap();

    println!("Proof generation finished. It took {:?}.", start.elapsed());

    // Save proof to file.
    proof_to_file(proof, "merger", None)
}

fn prove_merger_wrapper(initial_pks: &[G2MNT6], final_pks: &[G2MNT6], block_number: u32) {
    // Initialize rng.
    let rng = &mut thread_rng();

    // Load the proving key from file.
    let mut file = File::open(format!("proving_keys/merger_wrapper.bin")).unwrap();

    let proving_key = ProvingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the verifying key from file.
    let mut file = File::open(format!("verifying_keys/merger.bin")).unwrap();

    let vk_merger = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Load the proof from file.
    let mut file = File::open(format!("proofs/merger.bin")).unwrap();

    let proof = Proof::deserialize_unchecked(&mut file).unwrap();

    // Load the verifying key for Merger Wrapper from file.
    let mut file = File::open(format!("verifying_keys/merger_wrapper.bin")).unwrap();

    let vk_merger_wrapper = VerifyingKey::deserialize_unchecked(&mut file).unwrap();

    // Calculate the inputs.
    let initial_state_commitment = prepare_inputs(bytes_to_bits(&state_commitment(
        block_number,
        initial_pks.to_vec(),
    )));

    let final_state_commitment = prepare_inputs(bytes_to_bits(&state_commitment(
        block_number + EPOCH_LENGTH,
        final_pks.to_vec(),
    )));

    let vk_commitment = prepare_inputs(bytes_to_bits(&vk_commitment(vk_merger_wrapper)));

    // Create the proof.
    println!("Starting proof generation.");

    let start = Instant::now();

    let circuit = MergerWrapperCircuit::new(
        vk_merger,
        proof,
        initial_state_commitment,
        final_state_commitment,
        vk_commitment,
    );

    let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng).unwrap();

    println!("Proof generation finished. It took {:?}.", start.elapsed());

    // Save proof to file.
    proof_to_file(proof, "merger_wrapper", None)
}

fn proof_to_file<T: PairingEngine>(pk: Proof<T>, name: &str, number: Option<usize>) {
    // Save proof to file.
    println!("Storing proof.");

    if !Path::new("proofs/").is_dir() {
        DirBuilder::new().create("proofs/").unwrap();
    }

    let suffix = match number {
        None => "".to_string(),
        Some(n) => format!("_{}", n),
    };

    let mut file = File::create(format!("proofs/{}{}.bin", name, suffix)).unwrap();

    pk.serialize_unchecked(&mut file).unwrap();

    file.sync_all().unwrap();
}

fn state_comms_to_file(initial_state_commitment: Vec<u8>, final_state_commitment: Vec<u8>) {
    // Save inputs to file.
    println!("Storing inputs.");

    if !Path::new("proofs/").is_dir() {
        DirBuilder::new().create("proofs/").unwrap();
    }

    let mut file = File::create(format!("proofs/state_epoch_0.bin")).unwrap();

    initial_state_commitment
        .serialize_unchecked(&mut file)
        .unwrap();

    final_state_commitment
        .serialize_unchecked(&mut file)
        .unwrap();

    file.sync_all().unwrap();
}
