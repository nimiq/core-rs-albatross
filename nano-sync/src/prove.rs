use std::fs;
use std::fs::{DirBuilder, File};
use std::path::Path;

use ark_crypto_primitives::SNARK;
use ark_ec::{PairingEngine, ProjectiveCurve};
use ark_ff::Zero;
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_mnt4_753::MNT4_753;
use ark_mnt6_753::{G1Projective as G1MNT6, G2Projective as G2MNT6, MNT6_753};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::UniformRand;
use rand::{thread_rng, CryptoRng, Rng};

use nimiq_bls::pedersen::{pedersen_generators, pedersen_hash};

use crate::circuits::mnt4::{
    MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit as LeafMNT4, PKTreeNodeCircuit as NodeMNT4,
};
use crate::circuits::mnt6::{
    MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as NodeMNT6,
};
use crate::constants::{EPOCH_LENGTH, PK_TREE_BREADTH, PK_TREE_DEPTH, VALIDATOR_SLOTS};
use crate::primitives::{
    merkle_tree_prove, pk_tree_construct, serialize_g1_mnt6, serialize_g2_mnt6, state_commitment,
    vk_commitment, MacroBlock,
};
use crate::utils::{byte_to_le_bits, bytes_to_bits, pack_inputs};
use crate::{NanoZKP, NanoZKPError};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem};

impl NanoZKP {
    /// This function generates a proof for a new epoch, it uses the entire nano sync program. Note
    /// that the proof generation can easily take longer than 12 hours.
    pub fn prove(
        // The public keys of the validators of the initial state. So, the validators that were
        // selected in the previous election macro block and that are now signing this election
        // macro block.
        initial_pks: Vec<G2MNT6>,
        // The public keys of the validators of the final state. To be clear, they are the validators
        // that are selected in this election macro block.
        final_pks: Vec<G2MNT6>,
        // The current election macro block.
        block: MacroBlock,
        // If this is not the first epoch, you need to provide the SNARK proof for the previous
        // epoch and the genesis state commitment.
        genesis_data: Option<(Proof<MNT6_753>, Vec<u8>)>,
    ) -> Result<Proof<MNT6_753>, NanoZKPError> {
        // This is a flag indicating if we want to check the constraint system before creating the
        // proofs. It takes longer but is useful for debugging.
        let constraint_check = true;

        // This is a flag indicating if we want to cache the proofs. If true, we will see which proofs
        // were already created and start from there. Note that for this to work, you must provide
        // the exact same inputs.
        let proof_caching = true;

        let rng = &mut thread_rng();

        // Serialize the initial public keys into bits and chunk them into the number of leaves.
        let mut bytes = Vec::new();

        for i in 0..initial_pks.len() {
            bytes.extend_from_slice(&serialize_g2_mnt6(initial_pks[i]));
        }

        let bits = bytes_to_bits(&bytes);

        let mut pks_bits = Vec::new();

        for i in 0..PK_TREE_BREADTH {
            pks_bits.push(
                bits[i * bits.len() / PK_TREE_BREADTH..(i + 1) * bits.len() / PK_TREE_BREADTH]
                    .to_vec(),
            );
        }

        // Calculate the Merkle proofs for each leaf of the initial public key tree.
        let mut pk_tree_proofs = vec![];

        for i in 0..PK_TREE_BREADTH {
            let mut path = byte_to_le_bits(i as u8);

            path.truncate(PK_TREE_DEPTH);

            pk_tree_proofs.push(merkle_tree_prove(pks_bits.clone(), path));
        }

        // Calculate initial public key tree root.
        let initial_pk_tree_root = pk_tree_construct(initial_pks.clone());

        // Calculate final public key tree root.
        let final_pk_tree_root = pk_tree_construct(final_pks.clone());

        // Start generating proofs for PKTree level 5.
        for i in 0..32 {
            if proof_caching && Path::new(&format!("proofs/pk_tree_5_{}.bin", i)).exists() {
                continue;
            }

            println!("generating pk_tree_5_{}", i);

            NanoZKP::prove_pk_tree_leaf(
                rng,
                "pk_tree_5",
                i,
                &initial_pks,
                &pk_tree_proofs[i],
                &initial_pk_tree_root,
                &block.signer_bitmap,
                constraint_check,
            )?;
        }

        // Start generating proofs for PKTree level 4.
        for i in 0..16 {
            if proof_caching && Path::new(&format!("proofs/pk_tree_4_{}.bin", i)).exists() {
                continue;
            }

            println!("generating pk_tree_4_{}", i);

            NanoZKP::prove_pk_tree_node_mnt6(
                rng,
                "pk_tree_4",
                i,
                4,
                "pk_tree_5",
                &initial_pks,
                &initial_pk_tree_root,
                &block.signer_bitmap,
                constraint_check,
            )?;
        }

        // Start generating proofs for PKTree level 3.
        for i in 0..8 {
            if proof_caching && Path::new(&format!("proofs/pk_tree_3_{}.bin", i)).exists() {
                continue;
            }

            println!("generating pk_tree_3_{}", i);

            NanoZKP::prove_pk_tree_node_mnt4(
                rng,
                "pk_tree_3",
                i,
                3,
                "pk_tree_4",
                &initial_pks,
                &initial_pk_tree_root,
                &block.signer_bitmap,
                constraint_check,
            )?;
        }

        // Start generating proofs for PKTree level 2.
        for i in 0..4 {
            if proof_caching && Path::new(&format!("proofs/pk_tree_2_{}.bin", i)).exists() {
                continue;
            }

            println!("generating pk_tree_2_{}", i);

            NanoZKP::prove_pk_tree_node_mnt6(
                rng,
                "pk_tree_2",
                i,
                2,
                "pk_tree_3",
                &initial_pks,
                &initial_pk_tree_root,
                &block.signer_bitmap,
                constraint_check,
            )?;
        }

        // Start generating proofs for PKTree level 1.
        for i in 0..2 {
            if proof_caching && Path::new(&format!("proofs/pk_tree_1_{}.bin", i)).exists() {
                continue;
            }

            println!("generating pk_tree_1_{}", i);

            NanoZKP::prove_pk_tree_node_mnt4(
                rng,
                "pk_tree_1",
                i,
                1,
                "pk_tree_2",
                &initial_pks,
                &initial_pk_tree_root,
                &block.signer_bitmap,
                constraint_check,
            )?;
        }

        // Start generating proof for PKTree level 0.
        if !(proof_caching && Path::new("proofs/pk_tree_0_0.bin").exists()) {
            println!("generating pk_tree_0_0");

            NanoZKP::prove_pk_tree_node_mnt6(
                rng,
                "pk_tree_0",
                0,
                0,
                "pk_tree_1",
                &initial_pks,
                &initial_pk_tree_root,
                &block.signer_bitmap,
                constraint_check,
            )?;
        }

        // Start generating proof for Macro Block.
        if !(proof_caching && Path::new("proofs/macro_block.bin").exists()) {
            println!("generating macro_block");

            NanoZKP::prove_macro_block(
                rng,
                &initial_pks,
                &initial_pk_tree_root,
                &final_pks,
                &final_pk_tree_root,
                block.clone(),
                constraint_check,
            )?;
        }

        // Start generating proof for Macro Block Wrapper.
        if !(proof_caching && Path::new("proofs/macro_block_wrapper.bin").exists()) {
            println!("generating macro_block_wrapper");

            NanoZKP::prove_macro_block_wrapper(
                rng,
                &initial_pks,
                &final_pks,
                block.block_number,
                constraint_check,
            )?;
        }

        // Start generating proof for Merger.
        if !(proof_caching && Path::new("proofs/merger.bin").exists()) {
            println!("generating merger");

            NanoZKP::prove_merger(
                rng,
                &initial_pks,
                &final_pks,
                block.block_number,
                genesis_data.clone(),
                constraint_check,
            )?;
        }

        // Start generating proof for Merger Wrapper.
        println!("generating merger wrapper");

        let proof = NanoZKP::prove_merger_wrapper(
            rng,
            &initial_pks,
            &final_pks,
            block.block_number,
            genesis_data,
            constraint_check,
        )?;

        // Delete cached proofs.
        fs::remove_dir_all("proofs/")?;

        // Return proof.
        Ok(proof)
    }

    fn prove_pk_tree_leaf<R: CryptoRng + Rng>(
        rng: &mut R,
        name: &str,
        position: usize,
        pks: &[G2MNT6],
        pk_tree_nodes: &Vec<G1MNT6>,
        pk_tree_root: &Vec<u8>,
        signer_bitmap: &Vec<bool>,
        constraint_check: bool,
    ) -> Result<(), NanoZKPError> {
        // Load the proving key from file.
        let mut file = File::open(format!("proving_keys/{}.bin", name))?;

        let proving_key = ProvingKey::deserialize_unchecked(&mut file)?;

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
        let pk_tree_root = pack_inputs(bytes_to_bits(pk_tree_root));

        let agg_pk_commitment = pack_inputs(agg_pk_comm);

        let signer_bitmap = pack_inputs(signer_bitmap.clone()).pop().unwrap();

        let path = pack_inputs(byte_to_le_bits(position as u8)).pop().unwrap();

        // Create the circuit.
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

        // Optionally check the constraint system.
        if constraint_check {
            let cs = ConstraintSystem::new_ref();

            circuit.clone().generate_constraints(cs.clone()).unwrap();

            match cs.which_is_unsatisfied().unwrap() {
                None => {}
                Some(s) => {
                    println!("Unsatisfied @ {}", s);
                    assert!(false);
                }
            }
        }

        // Create the proof.
        let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, name, Some(position))
    }

    fn prove_pk_tree_node_mnt6<R: CryptoRng + Rng>(
        rng: &mut R,
        name: &str,
        position: usize,
        level: usize,
        vk_file: &str,
        pks: &[G2MNT6],
        pk_tree_root: &Vec<u8>,
        signer_bitmap: &Vec<bool>,
        constraint_check: bool,
    ) -> Result<(), NanoZKPError> {
        // Load the proving key from file.
        let mut file = File::open(format!("proving_keys/{}.bin", name))?;

        let proving_key = ProvingKey::deserialize_unchecked(&mut file)?;

        // Load the verifying key from file.
        let mut file = File::open(format!("verifying_keys/{}.bin", vk_file))?;

        let vk_child = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Load the left proof from file.
        let left_position = 2 * position;

        let mut file = File::open(format!("proofs/{}_{}.bin", vk_file, left_position))?;

        let left_proof = Proof::deserialize_unchecked(&mut file)?;

        // Load the right proof from file.
        let right_position = 2 * position + 1;

        let mut file = File::open(format!("proofs/{}_{}.bin", vk_file, right_position))?;

        let right_proof = Proof::deserialize_unchecked(&mut file)?;

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
        let pk_tree_root = pack_inputs(bytes_to_bits(pk_tree_root));

        let left_agg_pk_commitment = pack_inputs(left_agg_pk_comm);

        let right_agg_pk_commitment = pack_inputs(right_agg_pk_comm);

        let signer_bitmap = pack_inputs(signer_bitmap.clone()).pop().unwrap();

        let path = pack_inputs(byte_to_le_bits(position as u8)).pop().unwrap();

        // Create the circuit.
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

        // Optionally check the constraint system.
        if constraint_check {
            let cs = ConstraintSystem::new_ref();

            circuit.clone().generate_constraints(cs.clone()).unwrap();

            match cs.which_is_unsatisfied().unwrap() {
                None => {}
                Some(s) => {
                    println!("Unsatisfied @ {}", s);
                    assert!(false);
                }
            }
        }

        // Create the proof.
        let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng)?;

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, name, Some(position))
    }

    fn prove_pk_tree_node_mnt4<R: CryptoRng + Rng>(
        rng: &mut R,
        name: &str,
        position: usize,
        level: usize,
        vk_file: &str,
        pks: &[G2MNT6],
        pk_tree_root: &Vec<u8>,
        signer_bitmap: &Vec<bool>,
        constraint_check: bool,
    ) -> Result<(), NanoZKPError> {
        // Load the proving key from file.
        let mut file = File::open(format!("proving_keys/{}.bin", name))?;

        let proving_key = ProvingKey::deserialize_unchecked(&mut file)?;

        // Load the verifying key from file.
        let mut file = File::open(format!("verifying_keys/{}.bin", vk_file))?;

        let vk_child = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Load the left proof from file.
        let left_position = 2 * position;

        let mut file = File::open(format!("proofs/{}_{}.bin", vk_file, left_position))?;

        let left_proof = Proof::deserialize_unchecked(&mut file)?;

        // Load the right proof from file.
        let right_position = 2 * position + 1;

        let mut file = File::open(format!("proofs/{}_{}.bin", vk_file, right_position))?;

        let right_proof = Proof::deserialize_unchecked(&mut file)?;

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
        let pk_tree_root = pack_inputs(bytes_to_bits(pk_tree_root));

        let agg_pk_commitment = pack_inputs(agg_pk_comm);

        let signer_bitmap = pack_inputs(signer_bitmap.clone()).pop().unwrap();

        let path = pack_inputs(byte_to_le_bits(position as u8)).pop().unwrap();

        // Create the circuit.
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

        // Optionally check the constraint system.
        if constraint_check {
            let cs = ConstraintSystem::new_ref();

            circuit.clone().generate_constraints(cs.clone()).unwrap();

            match cs.which_is_unsatisfied().unwrap() {
                None => {}
                Some(s) => {
                    println!("Unsatisfied @ {}", s);
                    assert!(false);
                }
            }
        }

        // Create the proof.
        let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, name, Some(position))
    }

    fn prove_macro_block<R: CryptoRng + Rng>(
        rng: &mut R,
        initial_pks: &[G2MNT6],
        initial_pk_tree_root: &Vec<u8>,
        final_pks: &[G2MNT6],
        final_pk_tree_root: &Vec<u8>,
        block: MacroBlock,
        constraint_check: bool,
    ) -> Result<(), NanoZKPError> {
        // Load the proving key from file.
        let mut file = File::open("proving_keys/macro_block.bin".to_string())?;

        let proving_key = ProvingKey::deserialize_unchecked(&mut file)?;

        // Load the verifying key from file.
        let mut file = File::open("verifying_keys/pk_tree_0.bin".to_string())?;

        let vk_pk_tree = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Load the proof from file.
        let mut file = File::open("proofs/pk_tree_0_0.bin".to_string())?;

        let proof = Proof::deserialize_unchecked(&mut file)?;

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
        let initial_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block.block_number - EPOCH_LENGTH,
            initial_pks.to_vec(),
        )));

        let final_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block.block_number,
            final_pks.to_vec(),
        )));

        // Create the circuit.
        let circuit = MacroBlockCircuit::new(
            vk_pk_tree,
            agg_pk_chunks,
            proof,
            bytes_to_bits(initial_pk_tree_root),
            bytes_to_bits(final_pk_tree_root),
            block,
            initial_state_commitment,
            final_state_commitment,
        );

        // Optionally check the constraint system.
        if constraint_check {
            let cs = ConstraintSystem::new_ref();

            circuit.clone().generate_constraints(cs.clone()).unwrap();

            match cs.which_is_unsatisfied().unwrap() {
                None => {}
                Some(s) => {
                    println!("Unsatisfied @ {}", s);
                    assert!(false);
                }
            }
        }

        // Create the proof.
        let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, "macro_block", None)
    }

    fn prove_macro_block_wrapper<R: CryptoRng + Rng>(
        rng: &mut R,
        initial_pks: &[G2MNT6],
        final_pks: &[G2MNT6],
        block_number: u32,
        constraint_check: bool,
    ) -> Result<(), NanoZKPError> {
        // Load the proving key from file.
        let mut file = File::open("proving_keys/macro_block_wrapper.bin".to_string())?;

        let proving_key = ProvingKey::deserialize_unchecked(&mut file)?;

        // Load the verifying key from file.
        let mut file = File::open("verifying_keys/macro_block.bin".to_string())?;

        let vk_macro_block = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Load the proof from file.
        let mut file = File::open("proofs/macro_block.bin".to_string())?;

        let proof = Proof::deserialize_unchecked(&mut file)?;

        // Calculate the inputs.
        let initial_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block_number - EPOCH_LENGTH,
            initial_pks.to_vec(),
        )));

        let final_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block_number,
            final_pks.to_vec(),
        )));

        // Create the circuit.
        let circuit = MacroBlockWrapperCircuit::new(
            vk_macro_block,
            proof,
            initial_state_commitment,
            final_state_commitment,
        );

        // Optionally check the constraint system.
        if constraint_check {
            let cs = ConstraintSystem::new_ref();

            circuit.clone().generate_constraints(cs.clone()).unwrap();

            match cs.which_is_unsatisfied().unwrap() {
                None => {}
                Some(s) => {
                    println!("Unsatisfied @ {}", s);
                    assert!(false);
                }
            }
        }

        // Create the proof.
        let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng)?;

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, "macro_block_wrapper", None)
    }

    fn prove_merger<R: CryptoRng + Rng>(
        rng: &mut R,
        initial_pks: &[G2MNT6],
        final_pks: &[G2MNT6],
        block_number: u32,
        genesis_data: Option<(Proof<MNT6_753>, Vec<u8>)>,
        constraint_check: bool,
    ) -> Result<(), NanoZKPError> {
        // Load the proving key from file.
        let mut file = File::open("proving_keys/merger.bin".to_string())?;

        let proving_key = ProvingKey::deserialize_unchecked(&mut file)?;

        // Load the verifying key for Macro Block Wrapper from file.
        let mut file = File::open("verifying_keys/macro_block_wrapper.bin".to_string())?;

        let vk_macro_block_wrapper = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Load the proof for Macro Block Wrapper from file.
        let mut file = File::open("proofs/macro_block_wrapper.bin".to_string())?;

        let proof_macro_block_wrapper = Proof::deserialize_unchecked(&mut file)?;

        // Load the verifying key for Merger Wrapper from file.
        let mut file = File::open("verifying_keys/merger_wrapper.bin".to_string())?;

        let vk_merger_wrapper = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Get the intermediate state commitment.
        let intermediate_state_commitment =
            state_commitment(block_number - EPOCH_LENGTH, initial_pks.to_vec());

        // Create the proof for the previous epoch, the initial state commitment and the genesis flag
        // depending if this is the first epoch or not.
        let (proof_merger_wrapper, initial_state_comm_bytes, genesis_flag) = match genesis_data {
            None => (
                Proof {
                    a: G1MNT6::rand(rng).into_affine(),
                    b: G2MNT6::rand(rng).into_affine(),
                    c: G1MNT6::rand(rng).into_affine(),
                },
                intermediate_state_commitment.clone(),
                true,
            ),
            Some((proof, genesis_state)) => (proof, genesis_state, false),
        };

        // Calculate the inputs.
        let initial_state_commitment = pack_inputs(bytes_to_bits(&initial_state_comm_bytes));

        let final_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block_number,
            final_pks.to_vec(),
        )));

        let vk_commitment = pack_inputs(bytes_to_bits(&vk_commitment(vk_merger_wrapper.clone())));

        // Create the circuit.
        let circuit = MergerCircuit::new(
            vk_macro_block_wrapper,
            proof_merger_wrapper,
            proof_macro_block_wrapper,
            vk_merger_wrapper,
            bytes_to_bits(&intermediate_state_commitment),
            genesis_flag,
            initial_state_commitment,
            final_state_commitment,
            vk_commitment,
        );

        // Optionally check the constraint system.
        if constraint_check {
            let cs = ConstraintSystem::new_ref();

            circuit.clone().generate_constraints(cs.clone()).unwrap();

            match cs.which_is_unsatisfied().unwrap() {
                None => {}
                Some(s) => {
                    println!("Unsatisfied @ {}", s);
                    assert!(false);
                }
            }
        }

        // Create the proof.
        let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, "merger", None)
    }

    fn prove_merger_wrapper<R: CryptoRng + Rng>(
        rng: &mut R,
        initial_pks: &[G2MNT6],
        final_pks: &[G2MNT6],
        block_number: u32,
        genesis_data: Option<(Proof<MNT6_753>, Vec<u8>)>,
        constraint_check: bool,
    ) -> Result<Proof<MNT6_753>, NanoZKPError> {
        // Load the proving key from file.
        let mut file = File::open("proving_keys/merger_wrapper.bin".to_string())?;

        let proving_key = ProvingKey::deserialize_unchecked(&mut file)?;

        // Load the verifying key from file.
        let mut file = File::open("verifying_keys/merger.bin".to_string())?;

        let vk_merger = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Load the proof from file.
        let mut file = File::open("proofs/merger.bin".to_string())?;

        let proof = Proof::deserialize_unchecked(&mut file)?;

        // Load the verifying key for Merger Wrapper from file.
        let mut file = File::open("verifying_keys/merger_wrapper.bin".to_string())?;

        let vk_merger_wrapper = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Calculate the inputs.
        let initial_state_comm_bytes = match genesis_data {
            None => state_commitment(block_number - EPOCH_LENGTH, initial_pks.to_vec()),
            Some((_, x)) => x,
        };

        let initial_state_commitment = pack_inputs(bytes_to_bits(&initial_state_comm_bytes));

        let final_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block_number,
            final_pks.to_vec(),
        )));

        let vk_commitment = pack_inputs(bytes_to_bits(&vk_commitment(vk_merger_wrapper)));

        // Create the circuit.
        let circuit = MergerWrapperCircuit::new(
            vk_merger,
            proof,
            initial_state_commitment,
            final_state_commitment,
            vk_commitment,
        );

        // Optionally check the constraint system.
        if constraint_check {
            let cs = ConstraintSystem::new_ref();

            circuit.clone().generate_constraints(cs.clone()).unwrap();

            match cs.which_is_unsatisfied().unwrap() {
                None => {}
                Some(s) => {
                    println!("Unsatisfied @ {}", s);
                    assert!(false);
                }
            }
        }

        // Create the proof.
        let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng)?;

        // Cache proof to file.
        NanoZKP::proof_to_file(proof.clone(), "merger_wrapper", None)?;

        Ok(proof)
    }

    // Cache proof to file.
    fn proof_to_file<T: PairingEngine>(
        pk: Proof<T>,
        name: &str,
        number: Option<usize>,
    ) -> Result<(), NanoZKPError> {
        if !Path::new("proofs/").is_dir() {
            DirBuilder::new().create("proofs/")?;
        }

        let suffix = match number {
            None => "".to_string(),
            Some(n) => format!("_{}", n),
        };

        let mut file = File::create(format!("proofs/{}{}.bin", name, suffix))?;

        pk.serialize_unchecked(&mut file)?;

        file.sync_all()?;

        Ok(())
    }
}
