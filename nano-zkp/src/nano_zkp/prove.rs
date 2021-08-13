use std::fs;
use std::fs::{DirBuilder, File};
use std::path::Path;

use ark_crypto_primitives::SNARK;
use ark_ec::{PairingEngine, ProjectiveCurve};
use ark_ff::Zero;
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_mnt4_753::{Fr as MNT4Fr, MNT4_753};
use ark_mnt6_753::{Fr as MNT6Fr, G1Projective as G1MNT6, G2Projective as G2MNT6, MNT6_753};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::UniformRand;
use rand::{thread_rng, CryptoRng, Rng};

use nimiq_bls::pedersen::{pedersen_generators, pedersen_hash};
use nimiq_bls::utils::{byte_to_le_bits, bytes_to_bits};
use nimiq_nano_primitives::{merkle_tree_prove, serialize_g1_mnt6, serialize_g2_mnt6};
use nimiq_nano_primitives::{
    pk_tree_construct, state_commitment, vk_commitment, MacroBlock, PK_TREE_BREADTH, PK_TREE_DEPTH,
};
use nimiq_primitives::policy::{EPOCH_LENGTH, SLOTS};

use crate::circuits::mnt4::{
    MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit as LeafMNT4, PKTreeNodeCircuit as NodeMNT4,
};
use crate::circuits::mnt6::{
    MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as NodeMNT6,
};
use crate::utils::pack_inputs;
use crate::{NanoZKP, NanoZKPError};

impl NanoZKP {
    /// This function generates a proof for a new epoch, it uses the entire nano sync program. Note
    /// that the proof generation can easily take longer than 12 hours.
    pub fn prove(
        // The public keys of the validators of the initial state. So, the validators that were
        // selected in the previous election macro block and that are now signing this election
        // macro block.
        initial_pks: Vec<G2MNT6>,
        // The hash of the block header of the initial state. So, the hash of the block when the
        // initial validators were selected.
        initial_header_hash: [u8; 32],
        // The public keys of the validators of the final state. To be clear, they are the validators
        // that are selected in this election macro block.
        final_pks: Vec<G2MNT6>,
        // The current election macro block.
        block: MacroBlock,
        // If this is not the first epoch, you need to provide the SNARK proof for the previous
        // epoch and the genesis state commitment.
        genesis_data: Option<(Proof<MNT6_753>, Vec<u8>)>,
        // This is a flag indicating if we want to cache the proofs. If true, it will see which proofs
        // were already created and start from there. Note that for this to work, you must provide
        // the exact same inputs.
        proof_caching: bool,
        // This is a flag indicating if we want to run this function in debug mode. It will verify
        // each proof it creates right after the proof is generated.
        debug_mode: bool,
    ) -> Result<Proof<MNT6_753>, NanoZKPError> {
        let rng = &mut thread_rng();

        // Serialize the initial public keys into bits and chunk them into the number of leaves.
        let mut bytes = Vec::new();

        for initial_pk in &initial_pks {
            bytes.extend_from_slice(&serialize_g2_mnt6(initial_pk));
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
        #[allow(clippy::needless_range_loop)]
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
                debug_mode,
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
                debug_mode,
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
                debug_mode,
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
                debug_mode,
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
                debug_mode,
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
                debug_mode,
            )?;
        }

        // Start generating proof for Macro Block.
        if !(proof_caching && Path::new("proofs/macro_block.bin").exists()) {
            println!("generating macro_block");

            NanoZKP::prove_macro_block(
                rng,
                &initial_pks,
                &initial_pk_tree_root,
                initial_header_hash,
                &final_pks,
                &final_pk_tree_root,
                &block,
                debug_mode,
            )?;
        }

        // Start generating proof for Macro Block Wrapper.
        if !(proof_caching && Path::new("proofs/macro_block_wrapper.bin").exists()) {
            println!("generating macro_block_wrapper");

            NanoZKP::prove_macro_block_wrapper(
                rng,
                &initial_pks,
                initial_header_hash,
                &final_pks,
                &block,
                debug_mode,
            )?;
        }

        // Start generating proof for Merger.
        if !(proof_caching && Path::new("proofs/merger.bin").exists()) {
            println!("generating merger");

            NanoZKP::prove_merger(
                rng,
                &initial_pks,
                initial_header_hash,
                &final_pks,
                &block,
                genesis_data.clone(),
                debug_mode,
            )?;
        }

        // Start generating proof for Merger Wrapper.
        println!("generating merger wrapper");

        let proof = NanoZKP::prove_merger_wrapper(
            rng,
            &initial_pks,
            initial_header_hash,
            &final_pks,
            &block,
            genesis_data,
            debug_mode,
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
        pk_tree_nodes: &[G1MNT6],
        pk_tree_root: &[u8],
        signer_bitmap: &[bool],
        debug_mode: bool,
    ) -> Result<(), NanoZKPError> {
        // Load the proving key from file.
        let mut file = File::open(format!("proving_keys/{}.bin", name))?;

        let proving_key = ProvingKey::deserialize_unchecked(&mut file)?;

        // Calculate the aggregate public key commitment.
        let mut agg_pk = G2MNT6::zero();

        for i in position * SLOTS as usize / PK_TREE_BREADTH
            ..(position + 1) * SLOTS as usize / PK_TREE_BREADTH
        {
            if signer_bitmap[i] {
                agg_pk += pks[i];
            }
        }

        let agg_pk_bits = bytes_to_bits(&serialize_g2_mnt6(&agg_pk));

        let hash = pedersen_hash(agg_pk_bits, pedersen_generators(5));

        let agg_pk_comm = bytes_to_bits(&serialize_g1_mnt6(&hash));

        // Get the relevant chunk of the signer's bitmap.
        let signer_bitmap_chunk = &signer_bitmap[position * SLOTS as usize / PK_TREE_BREADTH
            ..(position + 1) * SLOTS as usize / PK_TREE_BREADTH];

        // Calculate inputs.
        let mut pk_tree_root = pack_inputs(bytes_to_bits(pk_tree_root));

        let mut agg_pk_commitment = pack_inputs(agg_pk_comm);

        let signer_bitmap_chunk: MNT4Fr = pack_inputs(signer_bitmap_chunk.to_vec()).pop().unwrap();

        let path: MNT4Fr = pack_inputs(byte_to_le_bits(position as u8)).pop().unwrap();

        // Create the circuit.
        let circuit = LeafMNT4::new(
            pks[position * SLOTS as usize / PK_TREE_BREADTH
                ..(position + 1) * SLOTS as usize / PK_TREE_BREADTH]
                .to_vec(),
            pk_tree_nodes.to_vec(),
            pk_tree_root.clone(),
            agg_pk_commitment.clone(),
            signer_bitmap_chunk,
            path,
        );

        // Create the proof.
        let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

        // Optionally verify the proof.
        if debug_mode {
            // Load the proving key from file.
            let mut file = File::open(format!("verifying_keys/{}.bin", name))?;

            let verifying_key = VerifyingKey::deserialize_unchecked(&mut file)?;

            // Prepare the inputs.
            let mut inputs = vec![];

            inputs.append(&mut pk_tree_root);

            inputs.append(&mut agg_pk_commitment);

            inputs.push(signer_bitmap_chunk);

            inputs.push(path);

            // Verify proof.
            assert!(Groth16::<MNT4_753>::verify(
                &verifying_key,
                &inputs,
                &proof
            )?);
        }

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, name, Some(position))
    }

    fn prove_pk_tree_node_mnt6<R: CryptoRng + Rng>(
        rng: &mut R,
        name: &str,
        position: usize,
        tree_level: usize,
        vk_file: &str,
        pks: &[G2MNT6],
        pk_tree_root: &[u8],
        signer_bitmap: &[bool],
        debug_mode: bool,
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

        for i in left_position * SLOTS as usize / 2_usize.pow((tree_level + 1) as u32)
            ..(left_position + 1) * SLOTS as usize / 2_usize.pow((tree_level + 1) as u32)
        {
            if signer_bitmap[i] {
                agg_pk += pks[i];
            }
        }

        let agg_pk_bits = bytes_to_bits(&serialize_g2_mnt6(&agg_pk));

        let hash = pedersen_hash(agg_pk_bits, pedersen_generators(5));

        let left_agg_pk_comm = bytes_to_bits(&serialize_g1_mnt6(&hash));

        // Calculate the right aggregate public key commitment.
        let mut agg_pk = G2MNT6::zero();

        for i in right_position * SLOTS as usize / 2_usize.pow((tree_level + 1) as u32)
            ..(right_position + 1) * SLOTS as usize / 2_usize.pow((tree_level + 1) as u32)
        {
            if signer_bitmap[i] {
                agg_pk += pks[i];
            }
        }

        let agg_pk_bits = bytes_to_bits(&serialize_g2_mnt6(&agg_pk));

        let hash = pedersen_hash(agg_pk_bits, pedersen_generators(5));

        let right_agg_pk_comm = bytes_to_bits(&serialize_g1_mnt6(&hash));

        // Get the relevant chunk of the signer's bitmap.
        let signer_bitmap_chunk = &signer_bitmap[position * SLOTS as usize
            / 2_usize.pow(tree_level as u32)
            ..(position + 1) * SLOTS as usize / 2_usize.pow(tree_level as u32)];

        // Calculate inputs.
        let mut pk_tree_root = pack_inputs(bytes_to_bits(pk_tree_root));

        let mut left_agg_pk_commitment = pack_inputs(left_agg_pk_comm);

        let mut right_agg_pk_commitment = pack_inputs(right_agg_pk_comm);

        let signer_bitmap_chunk: MNT6Fr = pack_inputs(signer_bitmap_chunk.to_vec()).pop().unwrap();

        let path: MNT6Fr = pack_inputs(byte_to_le_bits(position as u8)).pop().unwrap();

        // Create the circuit.
        let circuit = NodeMNT6::new(
            tree_level,
            vk_child,
            left_proof,
            right_proof,
            pk_tree_root.clone(),
            left_agg_pk_commitment.clone(),
            right_agg_pk_commitment.clone(),
            signer_bitmap_chunk,
            path,
        );

        // Create the proof.
        let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng)?;

        // Optionally verify the proof.
        if debug_mode {
            // Load the proving key from file.
            let mut file = File::open(format!("verifying_keys/{}.bin", name))?;

            let verifying_key = VerifyingKey::deserialize_unchecked(&mut file)?;

            // Prepare the inputs.
            let mut inputs = vec![];

            inputs.append(&mut pk_tree_root);

            inputs.append(&mut left_agg_pk_commitment);

            inputs.append(&mut right_agg_pk_commitment);

            inputs.push(signer_bitmap_chunk);

            inputs.push(path);

            // Verify proof.
            assert!(Groth16::<MNT6_753>::verify(
                &verifying_key,
                &inputs,
                &proof
            )?);
        }

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, name, Some(position))
    }

    fn prove_pk_tree_node_mnt4<R: CryptoRng + Rng>(
        rng: &mut R,
        name: &str,
        position: usize,
        tree_level: usize,
        vk_file: &str,
        pks: &[G2MNT6],
        pk_tree_root: &[u8],
        signer_bitmap: &[bool],
        debug_mode: bool,
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

            for j in i * SLOTS as usize / 2_usize.pow((tree_level + 2) as u32)
                ..(i + 1) * SLOTS as usize / 2_usize.pow((tree_level + 2) as u32)
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

        let agg_pk_bits = bytes_to_bits(&serialize_g2_mnt6(&agg_pk));

        let hash = pedersen_hash(agg_pk_bits, pedersen_generators(5));

        let agg_pk_comm = bytes_to_bits(&serialize_g1_mnt6(&hash));

        // Get the relevant chunk of the signer's bitmap.
        let signer_bitmap_chunk = &signer_bitmap[position * SLOTS as usize
            / 2_usize.pow(tree_level as u32)
            ..(position + 1) * SLOTS as usize / 2_usize.pow(tree_level as u32)];

        // Calculate inputs.
        let mut pk_tree_root = pack_inputs(bytes_to_bits(pk_tree_root));

        let mut agg_pk_commitment = pack_inputs(agg_pk_comm);

        let signer_bitmap_chunk: MNT4Fr = pack_inputs(signer_bitmap_chunk.to_vec()).pop().unwrap();

        let path: MNT4Fr = pack_inputs(byte_to_le_bits(position as u8)).pop().unwrap();

        // Create the circuit.
        let circuit = NodeMNT4::new(
            tree_level,
            vk_child,
            left_proof,
            right_proof,
            agg_pk_chunks,
            pk_tree_root.clone(),
            agg_pk_commitment.clone(),
            signer_bitmap_chunk,
            path,
        );

        // Create the proof.
        let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

        // Optionally verify the proof.
        if debug_mode {
            // Load the proving key from file.
            let mut file = File::open(format!("verifying_keys/{}.bin", name))?;

            let verifying_key = VerifyingKey::deserialize_unchecked(&mut file)?;

            // Prepare the inputs.
            let mut inputs = vec![];

            inputs.append(&mut pk_tree_root);

            inputs.append(&mut agg_pk_commitment);

            inputs.push(signer_bitmap_chunk);

            inputs.push(path);

            // Verify proof.
            assert!(Groth16::<MNT4_753>::verify(
                &verifying_key,
                &inputs,
                &proof
            )?);
        }

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, name, Some(position))
    }

    fn prove_macro_block<R: CryptoRng + Rng>(
        rng: &mut R,
        initial_pks: &[G2MNT6],
        initial_pk_tree_root: &[u8],
        initial_header_hash: [u8; 32],
        final_pks: &[G2MNT6],
        final_pk_tree_root: &[u8],
        block: &MacroBlock,
        debug_mode: bool,
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

            #[allow(clippy::needless_range_loop)]
            for j in i * SLOTS as usize / 2..(i + 1) * SLOTS as usize / 2 {
                if block.signer_bitmap[j] {
                    agg_pk += initial_pks[j];
                }
            }

            agg_pk_chunks.push(agg_pk);
        }

        // Calculate the inputs.
        let mut initial_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block.block_number - EPOCH_LENGTH,
            initial_header_hash,
            initial_pks.to_vec(),
        )));

        let mut final_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block.block_number,
            block.header_hash,
            final_pks.to_vec(),
        )));

        // Create the circuit.
        let circuit = MacroBlockCircuit::new(
            vk_pk_tree,
            agg_pk_chunks,
            proof,
            bytes_to_bits(initial_pk_tree_root),
            bytes_to_bits(&initial_header_hash),
            bytes_to_bits(final_pk_tree_root),
            block.clone(),
            initial_state_commitment.clone(),
            final_state_commitment.clone(),
        );

        // Create the proof.
        let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

        // Optionally verify the proof.
        if debug_mode {
            // Load the proving key from file.
            let mut file = File::open("verifying_keys/macro_block.bin".to_string())?;

            let verifying_key = VerifyingKey::deserialize_unchecked(&mut file)?;

            // Prepare the inputs.
            let mut inputs = vec![];

            inputs.append(&mut initial_state_commitment);

            inputs.append(&mut final_state_commitment);

            // Verify proof.
            assert!(Groth16::<MNT4_753>::verify(
                &verifying_key,
                &inputs,
                &proof
            )?);
        }

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, "macro_block", None)
    }

    fn prove_macro_block_wrapper<R: CryptoRng + Rng>(
        rng: &mut R,
        initial_pks: &[G2MNT6],
        initial_header_hash: [u8; 32],
        final_pks: &[G2MNT6],
        block: &MacroBlock,
        debug_mode: bool,
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
        let mut initial_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block.block_number - EPOCH_LENGTH,
            initial_header_hash,
            initial_pks.to_vec(),
        )));

        let mut final_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block.block_number,
            block.header_hash,
            final_pks.to_vec(),
        )));

        // Create the circuit.
        let circuit = MacroBlockWrapperCircuit::new(
            vk_macro_block,
            proof,
            initial_state_commitment.clone(),
            final_state_commitment.clone(),
        );

        // Create the proof.
        let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng)?;

        // Optionally verify the proof.
        if debug_mode {
            // Load the proving key from file.
            let mut file = File::open("verifying_keys/macro_block_wrapper.bin".to_string())?;

            let verifying_key = VerifyingKey::deserialize_unchecked(&mut file)?;

            // Prepare the inputs.
            let mut inputs = vec![];

            inputs.append(&mut initial_state_commitment);

            inputs.append(&mut final_state_commitment);

            // Verify proof.
            assert!(Groth16::<MNT6_753>::verify(
                &verifying_key,
                &inputs,
                &proof
            )?);
        }

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, "macro_block_wrapper", None)
    }

    fn prove_merger<R: CryptoRng + Rng>(
        rng: &mut R,
        initial_pks: &[G2MNT6],
        initial_header_hash: [u8; 32],
        final_pks: &[G2MNT6],
        block: &MacroBlock,
        genesis_data: Option<(Proof<MNT6_753>, Vec<u8>)>,
        debug_mode: bool,
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
        let intermediate_state_commitment = state_commitment(
            block.block_number - EPOCH_LENGTH,
            initial_header_hash,
            initial_pks.to_vec(),
        );

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
        let mut initial_state_commitment = pack_inputs(bytes_to_bits(&initial_state_comm_bytes));

        let mut final_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block.block_number,
            block.header_hash,
            final_pks.to_vec(),
        )));

        let mut vk_commitment =
            pack_inputs(bytes_to_bits(&vk_commitment(vk_merger_wrapper.clone())));

        // Create the circuit.
        let circuit = MergerCircuit::new(
            vk_macro_block_wrapper,
            proof_merger_wrapper,
            proof_macro_block_wrapper,
            vk_merger_wrapper,
            bytes_to_bits(&intermediate_state_commitment),
            genesis_flag,
            initial_state_commitment.clone(),
            final_state_commitment.clone(),
            vk_commitment.clone(),
        );

        // Create the proof.
        let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

        // Optionally verify the proof.
        if debug_mode {
            // Load the proving key from file.
            let mut file = File::open("verifying_keys/merger.bin".to_string())?;

            let verifying_key = VerifyingKey::deserialize_unchecked(&mut file)?;

            // Prepare the inputs.
            let mut inputs = vec![];

            inputs.append(&mut initial_state_commitment);

            inputs.append(&mut final_state_commitment);

            inputs.append(&mut vk_commitment);

            // Verify proof.
            assert!(Groth16::<MNT4_753>::verify(
                &verifying_key,
                &inputs,
                &proof
            )?);
        }

        // Cache proof to file.
        NanoZKP::proof_to_file(proof, "merger", None)
    }

    fn prove_merger_wrapper<R: CryptoRng + Rng>(
        rng: &mut R,
        initial_pks: &[G2MNT6],
        initial_header_hash: [u8; 32],
        final_pks: &[G2MNT6],
        block: &MacroBlock,
        genesis_data: Option<(Proof<MNT6_753>, Vec<u8>)>,
        debug_mode: bool,
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
            None => state_commitment(
                block.block_number - EPOCH_LENGTH,
                initial_header_hash,
                initial_pks.to_vec(),
            ),
            Some((_, x)) => x,
        };

        let mut initial_state_commitment = pack_inputs(bytes_to_bits(&initial_state_comm_bytes));

        let mut final_state_commitment = pack_inputs(bytes_to_bits(&state_commitment(
            block.block_number,
            block.header_hash,
            final_pks.to_vec(),
        )));

        let mut vk_commitment = pack_inputs(bytes_to_bits(&vk_commitment(vk_merger_wrapper)));

        // Create the circuit.
        let circuit = MergerWrapperCircuit::new(
            vk_merger,
            proof,
            initial_state_commitment.clone(),
            final_state_commitment.clone(),
            vk_commitment.clone(),
        );

        // Create the proof.
        let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng)?;

        // Optionally verify the proof.
        if debug_mode {
            // Load the proving key from file.
            let mut file = File::open("verifying_keys/merger_wrapper.bin".to_string())?;

            let verifying_key = VerifyingKey::deserialize_unchecked(&mut file)?;

            // Prepare the inputs.
            let mut inputs = vec![];

            inputs.append(&mut initial_state_commitment);

            inputs.append(&mut final_state_commitment);

            inputs.append(&mut vk_commitment);

            // Verify proof.
            assert!(Groth16::<MNT6_753>::verify(
                &verifying_key,
                &inputs,
                &proof
            )?);
        }

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
