use std::fs::{DirBuilder, File};
use std::path::Path;
use std::time::Instant;

use ark_crypto_primitives::{CircuitSpecificSetupSNARK, SNARK};
use ark_ec::{PairingEngine, ProjectiveCurve};
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_mnt4_753::{
    FqParameters, Fr as MNT4Fr, G1Projective as G1MNT4, G2Projective as G2MNT4, MNT4_753,
};
use ark_mnt6_753::{
    Fr as MNT6Fr, G1Projective as G1MNT6, G1Projective, G2Projective as G2MNT6, MNT6_753,
};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::UniformRand;
use rand::{thread_rng, RngCore};

use ark_ec::models::short_weierstrass_jacobian::GroupProjective;
use ark_ff::{Fp768, Zero};
use ark_std::ops::MulAssign;
use ark_std::usize::MIN;
use nimiq_bls::pedersen::{pedersen_generators, pedersen_hash};
use nimiq_nano_sync::circuits::mnt4::{
    MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit as LeafMNT4, PKTreeNodeCircuit as NodeMNT4,
};
use nimiq_nano_sync::circuits::mnt6::{
    MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as NodeMNT6,
};
use nimiq_nano_sync::constants::{MIN_SIGNERS, PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS};
use nimiq_nano_sync::primitives::{
    merkle_tree_prove, pk_tree_construct, serialize_g1_mnt6, serialize_g2_mnt6, MacroBlock,
};
use nimiq_nano_sync::utils::{byte_from_le_bits, byte_to_le_bits, bytes_to_bits, prepare_inputs};
use rand::prelude::SliceRandom;

fn main() {
    println!("====== Proof generation for Nano Sync initiated ======");
    let start = Instant::now();

    println!("====== Generating random inputs ======");
    let (sks, pks, pk_tree_proofs, pk_tree_root, signer_bitmap, block_number, round_number, block) =
        generate_inputs();

    println!("====== Generating proofs ======");
    // Start generating proofs for PKTree level 5.
    for i in 0..32 {
        println!("PKTree 5 circuit - number {}:", i);
        prove_pk_tree_leaf(
            "pk_tree_5",
            i,
            &pks,
            &pk_tree_proofs[i],
            &pk_tree_root,
            &signer_bitmap,
        )
    }

    // Start generating proofs for PKTree level 4.
    for i in 0..16 {
        println!("PKTree 4 circuit - number {}:", i);
        prove_pk_tree_node_mnt6(
            "pk_tree_4",
            i,
            "pk_tree_5",
            &pks,
            &pk_tree_root,
            &signer_bitmap,
        )
    }

    // Start generating proofs for PKTree level 3.
    for i in 0..8 {
        println!("PKTree 3 circuit - number {}:", i);
        prove_pk_tree_node_mnt4(
            "pk_tree_3",
            i,
            "pk_tree_4",
            &pks,
            &pk_tree_root,
            &signer_bitmap,
        )
    }

    println!("====== Proof generation for Nano Sync finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());
}

fn generate_inputs() -> (
    Vec<MNT6Fr>,
    Vec<G2MNT6>,
    Vec<Vec<G1MNT6>>,
    Vec<u8>,
    Vec<bool>,
    u32,
    u32,
    MacroBlock,
) {
    let start = Instant::now();

    // Initialize rng.
    let rng = &mut thread_rng();

    // Create key pairs for all the validators.
    let mut sks = vec![];
    let mut pks = vec![];

    for _ in 0..VALIDATOR_SLOTS {
        let sk = MNT6Fr::rand(rng);
        let mut pk = G2MNT6::prime_subgroup_generator();
        pk.mul_assign(sk);
        sks.push(sk);
        pks.push(pk);
    }

    // Serialize the public keys into bits and chunk them into the number of leaves.
    let mut bytes = Vec::new();

    for i in 0..pks.len() {
        bytes.extend_from_slice(&serialize_g2_mnt6(pks[i].clone()));
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

    // Calculate public key tree root.
    let pk_tree_root = pk_tree_construct(pks.clone());

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
                sks[i].clone(),
                i,
                block_number,
                round_number,
                pk_tree_root.clone(),
            );
        }
    }

    println!(
        "Input generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    (
        sks,
        pks,
        pk_tree_proofs,
        pk_tree_root,
        signer_bitmap,
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

    println!(
        "Proof generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save proof to file.
    to_file(proof, name, Some(position))
}

fn prove_pk_tree_node_mnt6(
    name: &str,
    position: usize,
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

    for i in left_position * VALIDATOR_SLOTS / PK_TREE_BREADTH
        ..(left_position + 1) * VALIDATOR_SLOTS / PK_TREE_BREADTH
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

    for i in right_position * VALIDATOR_SLOTS / PK_TREE_BREADTH
        ..(right_position + 1) * VALIDATOR_SLOTS / PK_TREE_BREADTH
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

    println!(
        "Proof generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save proof to file.
    to_file(proof, name, Some(position))
}

fn prove_pk_tree_node_mnt4(
    name: &str,
    position: usize,
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

        for j in i * VALIDATOR_SLOTS / PK_TREE_BREADTH..(i + 1) * VALIDATOR_SLOTS / PK_TREE_BREADTH
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

    println!(
        "Proof generation finished. It took {:?} seconds.",
        start.elapsed()
    );

    // Save proof to file.
    to_file(proof, name, Some(position))
}

// fn gen_params_macro_block(vk_file: &'static str, name: &str) {
//     // Initialize rng.
//     let rng = &mut thread_rng();
//
//     // Create dummy inputs.
//     let agg_pk_chunks = vec![G2MNT6::rand(rng); 2];
//
//     let proof = Proof {
//         a: G1MNT6::rand(rng).into_affine(),
//         b: G2MNT6::rand(rng).into_affine(),
//         c: G1MNT6::rand(rng).into_affine(),
//     };
//
//     let mut bytes = [0u8; 95];
//     rng.fill_bytes(&mut bytes);
//     let initial_pk_tree_root = bytes_to_bits(&bytes);
//
//     let initial_block_number = u32::rand(rng);
//
//     let initial_round_number = u32::rand(rng);
//
//     let mut bytes = [0u8; 95];
//     rng.fill_bytes(&mut bytes);
//     let final_pk_tree_root = bytes_to_bits(&bytes);
//
//     let initial_state_commitment = vec![MNT4Fr::rand(rng); 2];
//
//     let final_state_commitment = vec![MNT4Fr::rand(rng); 2];
//
//     let mut header_hash = [0u8; 32];
//     rng.fill_bytes(&mut header_hash);
//
//     let signature = G1MNT6::rand(rng);
//
//     let mut bytes = [0u8; VALIDATOR_SLOTS / 8];
//     rng.fill_bytes(&mut bytes);
//     let signer_bitmap = bytes_to_bits(&bytes);
//
//     let block = MacroBlock {
//         header_hash,
//         signature,
//         signer_bitmap,
//     };
//
//     // Create parameters for our circuit
//     println!("Starting parameter generation.");
//
//     let start = Instant::now();
//
//     let circuit = MacroBlockCircuit::new(
//         vk_file,
//         agg_pk_chunks,
//         proof,
//         initial_pk_tree_root,
//         initial_block_number,
//         initial_round_number,
//         final_pk_tree_root,
//         block,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//
//     let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng).unwrap();
//
//     println!(
//         "Parameter generation finished. It took {:?} seconds.",
//         start.elapsed()
//     );
//
//     // Save keys to file.
//     to_file(pk, vk, name)
// }
//
// fn gen_params_macro_block_wrapper(vk_file: &'static str, name: &str) {
//     // Initialize rng.
//     let rng = &mut thread_rng();
//
//     // Create dummy inputs.
//     let proof = Proof {
//         a: G1MNT4::rand(rng).into_affine(),
//         b: G2MNT4::rand(rng).into_affine(),
//         c: G1MNT4::rand(rng).into_affine(),
//     };
//
//     let initial_state_commitment = vec![MNT6Fr::rand(rng); 2];
//
//     let final_state_commitment = vec![MNT6Fr::rand(rng); 2];
//
//     // Create parameters for our circuit
//     println!("Starting parameter generation.");
//
//     let start = Instant::now();
//
//     let circuit = MacroBlockWrapperCircuit::new(
//         vk_file,
//         proof,
//         initial_state_commitment,
//         final_state_commitment,
//     );
//
//     let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng).unwrap();
//
//     println!(
//         "Parameter generation finished. It took {:?} seconds.",
//         start.elapsed()
//     );
//
//     // Save keys to file.
//     to_file(pk, vk, name)
// }
//
// fn gen_params_merger(vk_file: &'static str, name: &str) {
//     // Initialize rng.
//     let rng = &mut thread_rng();
//
//     // Create dummy inputs.
//     let proof_merger_wrapper = Proof {
//         a: G1MNT6::rand(rng).into_affine(),
//         b: G2MNT6::rand(rng).into_affine(),
//         c: G1MNT6::rand(rng).into_affine(),
//     };
//
//     let proof_macro_block_wrapper = Proof {
//         a: G1MNT6::rand(rng).into_affine(),
//         b: G2MNT6::rand(rng).into_affine(),
//         c: G1MNT6::rand(rng).into_affine(),
//     };
//
//     let vk_merger_wrapper = VerifyingKey {
//         alpha_g1: G1MNT6::rand(rng).into_affine(),
//         beta_g2: G2MNT6::rand(rng).into_affine(),
//         gamma_g2: G2MNT6::rand(rng).into_affine(),
//         delta_g2: G2MNT6::rand(rng).into_affine(),
//         gamma_abc_g1: vec![G1MNT6::rand(rng).into_affine(); 7],
//     };
//
//     let mut bytes = [0u8; 95];
//     rng.fill_bytes(&mut bytes);
//     let intermediate_state_commitment = bytes_to_bits(&bytes);
//
//     let genesis_flag = bool::rand(rng);
//
//     let initial_state_commitment = vec![MNT4Fr::rand(rng); 2];
//
//     let final_state_commitment = vec![MNT4Fr::rand(rng); 2];
//
//     let vk_commitment = vec![MNT4Fr::rand(rng); 2];
//
//     // Create parameters for our circuit
//     println!("Starting parameter generation.");
//
//     let start = Instant::now();
//
//     let circuit = MergerCircuit::new(
//         vk_file,
//         proof_merger_wrapper,
//         proof_macro_block_wrapper,
//         vk_merger_wrapper,
//         intermediate_state_commitment,
//         genesis_flag,
//         initial_state_commitment,
//         final_state_commitment,
//         vk_commitment,
//     );
//
//     let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng).unwrap();
//
//     println!(
//         "Parameter generation finished. It took {:?} seconds.",
//         start.elapsed()
//     );
//
//     // Save keys to file.
//     to_file(pk, vk, name)
// }
//
// fn gen_params_merger_wrapper(vk_file: &'static str, name: &str) {
//     // Initialize rng.
//     let rng = &mut thread_rng();
//
//     // Create dummy inputs.
//     let proof = Proof {
//         a: G1MNT4::rand(rng).into_affine(),
//         b: G2MNT4::rand(rng).into_affine(),
//         c: G1MNT4::rand(rng).into_affine(),
//     };
//
//     let initial_state_commitment = vec![MNT6Fr::rand(rng); 2];
//
//     let final_state_commitment = vec![MNT6Fr::rand(rng); 2];
//
//     let vk_commitment = vec![MNT6Fr::rand(rng); 2];
//
//     // Create parameters for our circuit
//     println!("Starting parameter generation.");
//
//     let start = Instant::now();
//
//     let circuit = MergerWrapperCircuit::new(
//         vk_file,
//         proof,
//         initial_state_commitment,
//         final_state_commitment,
//         vk_commitment,
//     );
//
//     let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng).unwrap();
//
//     println!(
//         "Parameter generation finished. It took {:?} seconds.",
//         start.elapsed()
//     );
//
//     // Save keys to file.
//     to_file(pk, vk, name)
// }

fn to_file<T: PairingEngine>(pk: Proof<T>, name: &str, number: Option<usize>) {
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
