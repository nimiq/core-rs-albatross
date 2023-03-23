use std::fs;
use std::fs::{DirBuilder, File};
use std::path::Path;

use ark_crypto_primitives::snark::SNARK;
use ark_ec::{pairing::Pairing, CurveGroup};
use ark_ff::{ToConstraintField, Zero};
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_mnt4_753::{Fq as MNT4Fq, MNT4_753};
use ark_mnt6_753::{Fq as MNT6Fq, G1Projective as G1MNT6, G2Projective as G2MNT6, MNT6_753};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::UniformRand;
use rand::{thread_rng, CryptoRng, Rng};

use nimiq_primitives::policy::Policy;
use nimiq_zkp_circuits::{
    bits::BitVec,
    circuits::mnt4::{
        MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as NodeMNT4,
    },
    circuits::mnt6::{
        MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit as LeafMNT6,
        PKTreeNodeCircuit as NodeMNT6,
    },
};
use nimiq_zkp_primitives::pedersen::default_pedersen_hash;
use nimiq_zkp_primitives::{
    pk_tree_construct, serialize_g1_mnt6, serialize_g2_mnt6, state_commitment, vk_commitment,
    MacroBlock, NanoZKPError, PK_TREE_DEPTH,
};

/// This function generates a proof for a new epoch, it uses the entire light macro sync. Note
/// that the proof generation can easily take longer than 12 hours.
pub fn prove(
    // The public keys of the validators of the previous state. So, the validators that were
    // selected in the previous election macro block and that are now signing this election
    // macro block.
    prev_pks: Vec<G2MNT6>,
    // The hash of the block header of the previous state. So, the hash of the block when the
    // previous validators were selected.
    prev_header_hash: [u8; 32],
    // The public keys of the validators of the final state. To be clear, they are the validators
    // that are selected in this election macro block.
    final_pks: Vec<G2MNT6>,
    // The current election macro block.
    block: MacroBlock,
    // If this is not the first epoch, you need to provide the SNARK proof for the previous
    // epoch and the genesis state commitment.
    genesis_data: Option<(Proof<MNT6_753>, [u8; 95])>,
    // This is a flag indicating if we want to cache the proofs. If true, it will see which proofs
    // were already created and start from there. Note that for this to work, you must provide
    // the exact same inputs.
    proof_caching: bool,
    // This is a flag indicating if we want to run this function in debug mode. It will verify
    // each proof it creates right after the proof is generated.
    debug_mode: bool,
    // The path to where the `prover_keys` folder is stored in.
    prover_keys_path: &Path,
) -> Result<Proof<MNT6_753>, NanoZKPError> {
    let rng = &mut thread_rng();
    let proofs = prover_keys_path.join("proofs");

    // Calculate previous public key tree root.
    let prev_pk_tree_root = pk_tree_construct(prev_pks.clone());

    // Calculate final public key tree root.
    let final_pk_tree_root = pk_tree_construct(final_pks);

    const NUM_PROOFS: usize = 4;
    let mut current_proof = 0;

    // Start generating proof for Macro Block.
    current_proof += 1;
    if !(proof_caching && proofs.join("macro_block.bin").exists()) {
        log::info!(
            "Generating sub-proof ({}/{}): macro_block",
            current_proof,
            NUM_PROOFS,
        );

        prove_macro_block(
            rng,
            &prev_pks,
            &prev_pk_tree_root,
            prev_header_hash,
            &final_pk_tree_root,
            &block,
            debug_mode,
            proof_caching,
            prover_keys_path,
        )?;
    }

    // Start generating proof for Macro Block Wrapper.
    current_proof += 1;
    if !(proof_caching && proofs.join("macro_block_wrapper.bin").exists()) {
        log::info!(
            "Generating sub-proof ({}/{}): macro_block_wrapper",
            current_proof,
            NUM_PROOFS,
        );

        prove_macro_block_wrapper(
            rng,
            &prev_pk_tree_root,
            prev_header_hash,
            &final_pk_tree_root,
            &block,
            debug_mode,
            prover_keys_path,
        )?;
    }

    // Start generating proof for Merger.
    current_proof += 1;
    if !(proof_caching && proofs.join("merger.bin").exists()) {
        log::info!(
            "Generating sub-proof ({}/{}): merger",
            current_proof,
            NUM_PROOFS,
        );

        prove_merger(
            rng,
            &prev_pk_tree_root,
            prev_header_hash,
            &final_pk_tree_root,
            &block,
            genesis_data.clone(),
            debug_mode,
            prover_keys_path,
        )?;
    }

    // Start generating proof for Merger Wrapper.
    current_proof += 1;
    log::info!(
        "Generating sub-proof ({}/{}): merger wrapper",
        current_proof,
        NUM_PROOFS,
    );

    let proof = prove_merger_wrapper(
        rng,
        &prev_pk_tree_root,
        prev_header_hash,
        &final_pk_tree_root,
        &block,
        genesis_data,
        debug_mode,
        prover_keys_path,
    )?;

    // Delete cached proofs.
    fs::remove_dir_all(proofs)?;

    // Return proof.
    Ok(proof)
}

fn prove_pk_tree_leaf<R: CryptoRng + Rng>(
    rng: &mut R,
    position: usize,
    pks: &[G2MNT6],
    signer_bitmap: &[bool],
    debug_mode: bool,
    proof_caching: bool,
    dir_path: &Path,
) -> Result<[u8; 95], NanoZKPError> {
    assert_eq!(pks.len(), signer_bitmap.len());

    let name = "pk_tree_5";

    // Calculate the aggregate public key commitment.
    let mut agg_pk = G2MNT6::zero();
    let mut pk_node_hash = vec![];

    for (i, pk) in pks.iter().enumerate() {
        pk_node_hash.extend(serialize_g2_mnt6(pk));
        if signer_bitmap[i] {
            agg_pk += pk;
        }
    }
    let hash = default_pedersen_hash(&pk_node_hash);
    let pk_node_hash = serialize_g1_mnt6(&hash);

    let agg_pk_bytes = serialize_g2_mnt6(&agg_pk);

    let hash = default_pedersen_hash(&agg_pk_bytes);
    let agg_pk_commitment = serialize_g1_mnt6(&hash);

    if proof_caching
        && dir_path
            .join("proofs")
            .join(format!("{name}_{position}.bin"))
            .exists()
    {
        return Ok(pk_node_hash);
    }

    log::info!("Generating sub-proof: {name}_{}", position);

    // Load the proving key from file.
    let mut file = File::open(dir_path.join("proving_keys").join(format!("{name}.bin")))?;
    let proving_key = ProvingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Create the circuit.
    let circuit = LeafMNT6::new(
        pks.to_vec(),
        pk_node_hash,
        agg_pk_commitment,
        signer_bitmap.to_vec(),
    );

    // Create the proof.
    let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

    // Optionally verify the proof.
    if debug_mode {
        // Load the verifying key from file.
        let mut file = File::open(dir_path.join("verifying_keys").join(format!("{name}.bin")))?;
        let verifying_key = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

        // Prepare the inputs.
        let mut inputs = vec![];
        inputs.append(&mut pk_node_hash.to_field_elements().unwrap());
        inputs.append(&mut agg_pk_commitment.to_field_elements().unwrap());
        inputs.append(
            &mut BitVec::<MNT6Fq>::to_bytes_le(signer_bitmap)
                .to_field_elements()
                .unwrap(),
        );

        // Verify proof.
        assert!(Groth16::<MNT4_753>::verify(
            &verifying_key,
            &inputs,
            &proof
        )?);
    }

    // Cache proof to file.
    proof_to_file(proof, name, Some(position), dir_path)?;

    Ok(pk_node_hash)
}

fn prove_pk_tree_node_mnt4<R: CryptoRng + Rng>(
    rng: &mut R,
    position: usize,
    tree_level: usize,
    pks: &[G2MNT6],
    signer_bitmap: &[bool],
    debug_mode: bool,
    proof_caching: bool,
    dir_path: &Path,
) -> Result<([u8; 95], [u8; 95]), NanoZKPError> {
    assert_eq!(pks.len(), signer_bitmap.len());

    let name = format!("pk_tree_{}", tree_level);
    let vk_file = format!("pk_tree_{}", tree_level + 1);

    let l_pks = &pks[..pks.len() / 2];
    let r_pks = &pks[pks.len() / 2..];
    let l_signer_bitmap = &signer_bitmap[..signer_bitmap.len() / 2];
    let r_signer_bitmap = &signer_bitmap[signer_bitmap.len() / 2..];

    // First create sub-proofs.
    let l_pk_node_hash;
    let r_pk_node_hash;
    if tree_level == PK_TREE_DEPTH - 1 {
        // Next level is the leaf node.
        l_pk_node_hash = prove_pk_tree_leaf(
            rng,
            2 * position,
            l_pks,
            l_signer_bitmap,
            debug_mode,
            proof_caching,
            dir_path,
        )?;

        r_pk_node_hash = prove_pk_tree_leaf(
            rng,
            2 * position + 1,
            r_pks,
            r_signer_bitmap,
            debug_mode,
            proof_caching,
            dir_path,
        )?;
    } else {
        // Next level is an inner node.
        l_pk_node_hash = prove_pk_tree_node_mnt6(
            rng,
            2 * position,
            tree_level + 1,
            l_pks,
            l_signer_bitmap,
            debug_mode,
            proof_caching,
            dir_path,
        )?;

        r_pk_node_hash = prove_pk_tree_node_mnt6(
            rng,
            2 * position + 1,
            tree_level + 1,
            r_pks,
            r_signer_bitmap,
            debug_mode,
            proof_caching,
            dir_path,
        )?;
    }

    let proving_keys = dir_path.join("proving_keys");
    let verifying_keys = dir_path.join("verifying_keys");
    let proofs = dir_path.join("proofs");

    if proof_caching && proofs.join(format!("{name}_{position}.bin")).exists() {
        return Ok((l_pk_node_hash, r_pk_node_hash));
    }

    log::info!("Generating sub-proof: {name}_{position}");

    // Load the proving key from file.
    let mut file = File::open(proving_keys.join(format!("{name}.bin")))?;
    let proving_key = ProvingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the verifying key from file.
    let mut file = File::open(verifying_keys.join(format!("{vk_file}.bin")))?;
    let vk_child = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the left proof from file.
    let left_position = 2 * position;

    let mut file = File::open(proofs.join(format!("{vk_file}_{left_position}.bin")))?;
    let left_proof = Proof::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the right proof from file.
    let right_position = 2 * position + 1;

    let mut file = File::open(proofs.join(format!("{vk_file}_{right_position}.bin")))?;
    let right_proof = Proof::deserialize_uncompressed_unchecked(&mut file)?;

    // Calculate the left aggregate public key commitment.
    let mut agg_pk = G2MNT6::zero();

    for (i, pk) in l_pks.iter().enumerate() {
        if l_signer_bitmap[i] {
            agg_pk += pk;
        }
    }

    let agg_pk_bytes = serialize_g2_mnt6(&agg_pk);
    let hash = default_pedersen_hash(&agg_pk_bytes);
    let left_agg_pk_comm = serialize_g1_mnt6(&hash);

    // Calculate the right aggregate public key commitment.
    let mut agg_pk = G2MNT6::zero();

    for (i, pk) in r_pks.iter().enumerate() {
        if r_signer_bitmap[i] {
            agg_pk += pk;
        }
    }

    let agg_pk_bytes = serialize_g2_mnt6(&agg_pk);
    let hash = default_pedersen_hash(&agg_pk_bytes);
    let right_agg_pk_comm = serialize_g1_mnt6(&hash);

    // Create the circuit.
    let circuit = NodeMNT4::new(
        tree_level,
        vk_child,
        left_proof,
        right_proof,
        l_pk_node_hash,
        r_pk_node_hash,
        left_agg_pk_comm,
        right_agg_pk_comm,
        signer_bitmap.to_vec(),
    );

    // Create the proof.
    let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng)?;

    // Optionally verify the proof.
    if debug_mode {
        // Load the verifying key from file.
        let mut file = File::open(verifying_keys.join(format!("{name}.bin")))?;
        let verifying_key = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

        // Prepare the inputs.
        let mut inputs = vec![];
        inputs.append(&mut l_pk_node_hash.to_field_elements().unwrap());
        inputs.append(&mut r_pk_node_hash.to_field_elements().unwrap());
        inputs.append(&mut left_agg_pk_comm.to_field_elements().unwrap());
        inputs.append(&mut right_agg_pk_comm.to_field_elements().unwrap());

        inputs.append(
            &mut BitVec::<MNT4Fq>::to_bytes_le(signer_bitmap)
                .to_field_elements()
                .unwrap(),
        );

        // Verify proof.
        assert!(Groth16::<MNT6_753>::verify(
            &verifying_key,
            &inputs,
            &proof
        )?);
    }

    // Cache proof to file.
    proof_to_file(proof, &name, Some(position), dir_path)?;
    Ok((l_pk_node_hash, r_pk_node_hash))
}

fn prove_pk_tree_node_mnt6<R: CryptoRng + Rng>(
    rng: &mut R,
    position: usize,
    tree_level: usize,
    pks: &[G2MNT6],
    signer_bitmap: &[bool],
    debug_mode: bool,
    proof_caching: bool,
    dir_path: &Path,
) -> Result<[u8; 95], NanoZKPError> {
    assert_eq!(pks.len(), signer_bitmap.len());

    let name = format!("pk_tree_{}", tree_level);
    let vk_file = format!("pk_tree_{}", tree_level + 1);

    let l_pks = &pks[..pks.len() / 2];
    let r_pks = &pks[pks.len() / 2..];
    let ll_pks = &l_pks[..l_pks.len() / 2];
    let lr_pks = &l_pks[l_pks.len() / 2..];
    let rl_pks = &r_pks[..r_pks.len() / 2];
    let rr_pks = &r_pks[r_pks.len() / 2..];
    let l_signer_bitmap = &signer_bitmap[..signer_bitmap.len() / 2];
    let r_signer_bitmap = &signer_bitmap[signer_bitmap.len() / 2..];
    let ll_signer_bitmap = &l_signer_bitmap[..l_signer_bitmap.len() / 2];
    let lr_signer_bitmap = &l_signer_bitmap[l_signer_bitmap.len() / 2..];
    let rl_signer_bitmap = &r_signer_bitmap[..r_signer_bitmap.len() / 2];
    let rr_signer_bitmap = &r_signer_bitmap[r_signer_bitmap.len() / 2..];

    let proving_keys = dir_path.join("proving_keys");
    let verifying_keys = dir_path.join("verifying_keys");
    let proofs = dir_path.join("proofs");

    // Next level is always an inner node.
    let (ll_pk_node_hash, lr_pk_node_hash) = prove_pk_tree_node_mnt4(
        rng,
        2 * position,
        tree_level + 1,
        l_pks,
        l_signer_bitmap,
        debug_mode,
        proof_caching,
        dir_path,
    )?;

    let (rl_pk_node_hash, rr_pk_node_hash) = prove_pk_tree_node_mnt4(
        rng,
        2 * position + 1,
        tree_level + 1,
        r_pks,
        r_signer_bitmap,
        debug_mode,
        proof_caching,
        dir_path,
    )?;

    // Calculate the node hash.
    let mut l_pk_node_hash = vec![];
    l_pk_node_hash.extend(ll_pk_node_hash);
    l_pk_node_hash.extend(lr_pk_node_hash);

    let hash = default_pedersen_hash(&l_pk_node_hash);
    let l_pk_node_hash = serialize_g1_mnt6(&hash);

    let mut r_pk_node_hash = vec![];
    r_pk_node_hash.extend(rl_pk_node_hash);
    r_pk_node_hash.extend(rr_pk_node_hash);

    let hash = default_pedersen_hash(&r_pk_node_hash);
    let r_pk_node_hash = serialize_g1_mnt6(&hash);

    let mut pk_node_hash = vec![];
    pk_node_hash.extend(l_pk_node_hash);
    pk_node_hash.extend(r_pk_node_hash);

    let hash = default_pedersen_hash(&pk_node_hash);
    let pk_node_hash = serialize_g1_mnt6(&hash);

    if proof_caching && proofs.join(format!("{name}_{position}.bin")).exists() {
        return Ok(pk_node_hash);
    }

    log::info!("Generating sub-proof: {name}_{position}");

    // Load the proving key from file.
    let mut file = File::open(proving_keys.join(format!("{name}.bin")))?;
    let proving_key = ProvingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the verifying key from file.
    let mut file = File::open(verifying_keys.join(format!("{vk_file}.bin")))?;
    let vk_child = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the left proof from file.
    let left_position = 2 * position;

    let mut file = File::open(proofs.join(format!("{vk_file}_{left_position}.bin")))?;
    let left_proof = Proof::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the right proof from file.
    let right_position = 2 * position + 1;

    let mut file = File::open(proofs.join(format!("{vk_file}_{right_position}.bin")))?;
    let right_proof = Proof::deserialize_uncompressed_unchecked(&mut file)?;

    // Calculate the aggregate public key chunks.
    let mut agg_pk_chunks = vec![];

    let mut agg_pk = G2MNT6::zero();
    for (i, pk) in ll_pks.iter().enumerate() {
        if ll_signer_bitmap[i] {
            agg_pk += pk;
        }
    }
    agg_pk_chunks.push(agg_pk);

    let mut agg_pk = G2MNT6::zero();
    for (i, pk) in lr_pks.iter().enumerate() {
        if lr_signer_bitmap[i] {
            agg_pk += pk;
        }
    }
    agg_pk_chunks.push(agg_pk);

    let mut agg_pk = G2MNT6::zero();
    for (i, pk) in rl_pks.iter().enumerate() {
        if rl_signer_bitmap[i] {
            agg_pk += pk;
        }
    }
    agg_pk_chunks.push(agg_pk);

    let mut agg_pk = G2MNT6::zero();
    for (i, pk) in rr_pks.iter().enumerate() {
        if rr_signer_bitmap[i] {
            agg_pk += pk;
        }
    }
    agg_pk_chunks.push(agg_pk);

    // Calculate the aggregate public key commitment.
    let mut agg_pk = G2MNT6::zero();

    for chunk in &agg_pk_chunks {
        agg_pk += chunk;
    }

    let agg_pk_bytes = serialize_g2_mnt6(&agg_pk);
    let hash = default_pedersen_hash(&agg_pk_bytes);
    let agg_pk_comm = serialize_g1_mnt6(&hash);

    // Create the circuit.
    let circuit = NodeMNT6::new(
        tree_level,
        vk_child,
        left_proof,
        right_proof,
        agg_pk_chunks[0],
        agg_pk_chunks[1],
        agg_pk_chunks[2],
        agg_pk_chunks[3],
        ll_pk_node_hash,
        lr_pk_node_hash,
        rl_pk_node_hash,
        rr_pk_node_hash,
        pk_node_hash,
        agg_pk_comm,
        signer_bitmap.to_vec(),
    );

    // Create the proof.
    let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

    // Optionally verify the proof.
    if debug_mode {
        // Load the verifying key from file.
        let mut file = File::open(verifying_keys.join(format!("{name}.bin")))?;
        let verifying_key = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

        // Prepare the inputs.
        let mut inputs = vec![];

        inputs.append(&mut pk_node_hash.to_field_elements().unwrap());
        inputs.append(&mut agg_pk_comm.to_field_elements().unwrap());
        inputs.append(
            &mut BitVec::<MNT6Fq>::to_bytes_le(signer_bitmap)
                .to_field_elements()
                .unwrap(),
        );

        // Verify proof.
        assert!(Groth16::<MNT4_753>::verify(
            &verifying_key,
            &inputs,
            &proof
        )?);
    }

    // Cache proof to file.
    proof_to_file(proof, &name, Some(position), dir_path)?;

    Ok(pk_node_hash)
}

fn prove_macro_block<R: CryptoRng + Rng>(
    rng: &mut R,
    prev_pks: &[G2MNT6],
    prev_pk_tree_root: &[u8; 95],
    prev_header_hash: [u8; 32],
    final_pk_tree_root: &[u8; 95],
    block: &MacroBlock,
    debug_mode: bool,
    proof_caching: bool,
    path: &Path,
) -> Result<(), NanoZKPError> {
    // Generate the PK Tree proofs.
    let (l_pk_node_hash, r_pk_node_hash) = prove_pk_tree_node_mnt4(
        rng,
        0,
        0,
        prev_pks,
        &block.signer_bitmap,
        debug_mode,
        proof_caching,
        path,
    )?;

    let proving_keys = path.join("proving_keys");
    let verifying_keys = path.join("verifying_keys");
    let proofs = path.join("proofs");

    // Load the proving key from file.
    let mut file = File::open(proving_keys.join("macro_block.bin"))?;
    let proving_key = ProvingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the verifying key from file.
    let mut file = File::open(verifying_keys.join("pk_tree_0.bin"))?;
    let vk_pk_tree = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the proof from file.
    let mut file = File::open(proofs.join("pk_tree_0_0.bin"))?;
    let proof = Proof::deserialize_uncompressed_unchecked(&mut file)?;

    // Calculate the aggregate public key chunks.
    let mut agg_pk_chunks = vec![];

    for i in 0..2 {
        let mut agg_pk = G2MNT6::zero();

        #[allow(clippy::needless_range_loop)]
        for j in i * Policy::SLOTS as usize / 2..(i + 1) * Policy::SLOTS as usize / 2 {
            if block.signer_bitmap[j] {
                agg_pk += prev_pks[j];
            }
        }

        agg_pk_chunks.push(agg_pk);
    }

    // Calculate the inputs.
    let prev_state_commitment = state_commitment(
        block.block_number - Policy::blocks_per_epoch(),
        &prev_header_hash,
        prev_pk_tree_root,
    );

    let final_state_commitment =
        state_commitment(block.block_number, &block.header_hash, final_pk_tree_root);

    // Create the circuit.
    let circuit = MacroBlockCircuit::new(
        vk_pk_tree,
        proof,
        *prev_pk_tree_root,
        prev_header_hash,
        *final_pk_tree_root,
        block.clone(),
        l_pk_node_hash,
        r_pk_node_hash,
        agg_pk_chunks[0],
        agg_pk_chunks[1],
        prev_state_commitment,
        final_state_commitment,
    );

    // Create the proof.
    let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

    // Optionally verify the proof.
    if debug_mode {
        // Load the verifying key from file.
        let mut file = File::open(verifying_keys.join("macro_block.bin"))?;
        let verifying_key = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

        // Prepare the inputs.
        let mut inputs = vec![];
        inputs.append(&mut prev_state_commitment.to_field_elements().unwrap());
        inputs.append(&mut final_state_commitment.to_field_elements().unwrap());

        // Verify proof.
        assert!(Groth16::<MNT4_753>::verify(
            &verifying_key,
            &inputs,
            &proof
        )?);
    }

    // Cache proof to file.
    proof_to_file(proof, "macro_block", None, path)
}

fn prove_macro_block_wrapper<R: CryptoRng + Rng>(
    rng: &mut R,
    prev_pk_tree_root: &[u8; 95],
    prev_header_hash: [u8; 32],
    final_pk_tree_root: &[u8; 95],
    block: &MacroBlock,
    debug_mode: bool,
    path: &Path,
) -> Result<(), NanoZKPError> {
    let proving_keys = path.join("proving_keys");
    let verifying_keys = path.join("verifying_keys");
    let proofs = path.join("proofs");

    // Load the proving key from file.
    let mut file = File::open(proving_keys.join("macro_block_wrapper.bin"))?;
    let proving_key = ProvingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the verifying key from file.
    let mut file = File::open(verifying_keys.join("macro_block.bin"))?;
    let vk_macro_block = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the proof from file.
    let mut file = File::open(proofs.join("macro_block.bin"))?;
    let proof = Proof::deserialize_uncompressed_unchecked(&mut file)?;

    // Calculate the inputs.
    let prev_state_commitment = state_commitment(
        block.block_number - Policy::blocks_per_epoch(),
        &prev_header_hash,
        prev_pk_tree_root,
    );

    let final_state_commitment =
        state_commitment(block.block_number, &block.header_hash, final_pk_tree_root);

    // Create the circuit.
    let circuit = MacroBlockWrapperCircuit::new(
        vk_macro_block,
        proof,
        prev_state_commitment,
        final_state_commitment,
    );

    // Create the proof.
    let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng)?;

    // Optionally verify the proof.
    if debug_mode {
        // Load the verifying key from file.
        let mut file = File::open(verifying_keys.join("macro_block_wrapper.bin"))?;
        let verifying_key = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

        // Prepare the inputs.
        let mut inputs = vec![];
        inputs.append(&mut prev_state_commitment.to_field_elements().unwrap());
        inputs.append(&mut final_state_commitment.to_field_elements().unwrap());

        // Verify proof.
        assert!(Groth16::<MNT6_753>::verify(
            &verifying_key,
            &inputs,
            &proof
        )?);
    }

    // Cache proof to file.
    proof_to_file(proof, "macro_block_wrapper", None, path)
}

fn prove_merger<R: CryptoRng + Rng>(
    rng: &mut R,
    prev_pk_tree_root: &[u8; 95],
    prev_header_hash: [u8; 32],
    final_pk_tree_root: &[u8; 95],
    block: &MacroBlock,
    genesis_data: Option<(Proof<MNT6_753>, [u8; 95])>,
    debug_mode: bool,
    path: &Path,
) -> Result<(), NanoZKPError> {
    let proving_keys = path.join("proving_keys");
    let verifying_keys = path.join("verifying_keys");
    let proofs = path.join("proofs");
    // Load the proving key from file.
    let mut file = File::open(proving_keys.join("merger.bin"))?;
    let proving_key = ProvingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the verifying key for Macro Block Wrapper from file.
    let mut file = File::open(verifying_keys.join("macro_block_wrapper.bin"))?;
    let vk_macro_block_wrapper = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the proof for Macro Block Wrapper from file.
    let mut file = File::open(proofs.join("macro_block_wrapper.bin"))?;
    let proof_macro_block_wrapper = Proof::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the verifying key for Merger Wrapper from file.
    let mut file = File::open(verifying_keys.join("merger_wrapper.bin"))?;
    let vk_merger_wrapper = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Get the intermediate state commitment.
    let intermediate_state_commitment = state_commitment(
        block.block_number - Policy::blocks_per_epoch(),
        &prev_header_hash,
        prev_pk_tree_root,
    );

    // Create the proof for the previous epoch, the genesis state commitment and the genesis flag
    // depending if this is the first epoch or not.
    let (proof_merger_wrapper, genesis_state_commitment, genesis_flag) = match genesis_data {
        None => (
            Proof {
                a: G1MNT6::rand(rng).into_affine(),
                b: G2MNT6::rand(rng).into_affine(),
                c: G1MNT6::rand(rng).into_affine(),
            },
            intermediate_state_commitment,
            true,
        ),
        Some((proof, genesis_state_commitment)) => (proof, genesis_state_commitment, false),
    };

    // Calculate the inputs.
    let final_state_commitment =
        state_commitment(block.block_number, &block.header_hash, final_pk_tree_root);

    let vk_commitment = vk_commitment(vk_merger_wrapper.clone());

    // Create the circuit.
    let circuit = MergerCircuit::new(
        vk_macro_block_wrapper,
        proof_merger_wrapper,
        proof_macro_block_wrapper,
        vk_merger_wrapper,
        intermediate_state_commitment,
        genesis_flag,
        genesis_state_commitment,
        final_state_commitment,
        vk_commitment,
    );

    // Create the proof.
    let proof = Groth16::<MNT4_753>::prove(&proving_key, circuit, rng)?;

    // Optionally verify the proof.
    if debug_mode {
        // Load the verifying key from file.
        let mut file = File::open(verifying_keys.join("merger.bin"))?;
        let verifying_key = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

        // Prepare the inputs.
        let mut inputs = vec![];
        inputs.append(&mut genesis_state_commitment.to_field_elements().unwrap());
        inputs.append(&mut final_state_commitment.to_field_elements().unwrap());
        inputs.append(&mut vk_commitment.to_field_elements().unwrap());

        // Verify proof.
        assert!(Groth16::<MNT4_753>::verify(
            &verifying_key,
            &inputs,
            &proof
        )?);
    }

    // Cache proof to file.
    proof_to_file(proof, "merger", None, path)
}

fn prove_merger_wrapper<R: CryptoRng + Rng>(
    rng: &mut R,
    prev_pk_tree_root: &[u8; 95],
    prev_header_hash: [u8; 32],
    final_pk_tree_root: &[u8; 95],
    block: &MacroBlock,
    genesis_data: Option<(Proof<MNT6_753>, [u8; 95])>,
    debug_mode: bool,
    path: &Path,
) -> Result<Proof<MNT6_753>, NanoZKPError> {
    let proving_keys = path.join("proving_keys");
    let verifying_keys = path.join("verifying_keys");
    let proofs = path.join("proofs");
    // Load the proving key from file.
    let mut file = File::open(proving_keys.join("merger_wrapper.bin"))?;
    let proving_key = ProvingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the verifying key from file.
    let mut file = File::open(verifying_keys.join("merger.bin"))?;
    let vk_merger = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the proof from file.
    let mut file = File::open(proofs.join("merger.bin"))?;
    let proof = Proof::deserialize_uncompressed_unchecked(&mut file)?;

    // Load the verifying key for Merger Wrapper from file.
    let mut file = File::open(verifying_keys.join("merger_wrapper.bin"))?;
    let vk_merger_wrapper = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Calculate the inputs.
    let genesis_state_commitment = match genesis_data {
        None => state_commitment(
            block.block_number - Policy::blocks_per_epoch(),
            &prev_header_hash,
            prev_pk_tree_root,
        ),
        Some((_, x)) => x,
    };

    let final_state_commitment =
        state_commitment(block.block_number, &block.header_hash, final_pk_tree_root);

    let vk_commitment = vk_commitment(vk_merger_wrapper);

    // Create the circuit.
    let circuit = MergerWrapperCircuit::new(
        vk_merger,
        proof,
        genesis_state_commitment,
        final_state_commitment,
        vk_commitment,
    );

    // Create the proof.
    let proof = Groth16::<MNT6_753>::prove(&proving_key, circuit, rng)?;

    // Optionally verify the proof.
    if debug_mode {
        // Load the verifying key from file.
        let mut file = File::open(verifying_keys.join("merger_wrapper.bin"))?;
        let verifying_key = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

        // Prepare the inputs.
        let mut inputs = vec![];
        inputs.append(&mut genesis_state_commitment.to_field_elements().unwrap());
        inputs.append(&mut final_state_commitment.to_field_elements().unwrap());
        inputs.append(&mut vk_commitment.to_field_elements().unwrap());

        // Verify proof.
        assert!(Groth16::<MNT6_753>::verify(
            &verifying_key,
            &inputs,
            &proof
        )?);
    }

    // Cache proof to file.
    proof_to_file(proof.clone(), "merger_wrapper", None, path)?;

    Ok(proof)
}

// Cache proof to file.
fn proof_to_file<T: Pairing>(
    pk: Proof<T>,
    name: &str,
    number: Option<usize>,
    path: &Path,
) -> Result<(), NanoZKPError> {
    let proofs = path.join("proofs");
    if !proofs.is_dir() {
        DirBuilder::new().recursive(true).create(&proofs)?;
    }

    let suffix = match number {
        None => "".to_string(),
        Some(n) => format!("_{n}"),
    };

    let mut file = File::create(proofs.join(format!("{name}{suffix}.bin")))?;
    pk.serialize_uncompressed(&mut file)?;
    file.sync_all()?;

    Ok(())
}
