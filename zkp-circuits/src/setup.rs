use std::{
    fs::{self, DirBuilder, File},
    path::Path,
};

use ark_crypto_primitives::snark::CircuitSpecificSetupSNARK;
use ark_ec::pairing::Pairing;
use ark_groth16::{Groth16, ProvingKey, VerifyingKey};
use ark_mnt4_753::MNT4_753;
use ark_mnt6_753::MNT6_753;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use nimiq_genesis::NetworkInfo;
use nimiq_primitives::networks::NetworkId;
use nimiq_serde::Deserialize;
use nimiq_zkp_primitives::{NanoZKPError, VerifyingData};
use rand::{CryptoRng, Rng};

use crate::{
    circuits::{
        mnt4::{MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as NodeMNT4},
        mnt6::{
            MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit as LeafMNT6,
            PKTreeNodeCircuit as NodeMNT6,
        },
        vk_commitments::VerifyingKeys,
        CircuitInput,
    },
    metadata::VerifyingKeyMetadata,
};

pub const DEVELOPMENT_SEED: [u8; 32] = [
    1, 0, 52, 0, 0, 0, 0, 0, 1, 0, 10, 0, 22, 32, 0, 0, 2, 0, 55, 49, 0, 11, 0, 0, 3, 0, 0, 0, 0,
    0, 2, 92,
];

/// This function generates the parameters (proving and verifying keys) for the entire light macro sync.
/// It does this by generating the parameters for each circuit, "from bottom to top". The
/// order is absolutely necessary because each circuit needs a verifying key from the circuit "below"
/// it. Note that the parameter generation can take longer than one hour, even two on some computers.
pub fn setup<R: Rng + CryptoRng>(
    mut rng: R,
    path: &Path,
    network_id: NetworkId,
    prover_active: bool,
) -> Result<(), NanoZKPError> {
    if all_files_created(path, prover_active) {
        save_metadata_to_file(path, network_id)?;

        return Ok(());
    }

    setup_pk_tree_leaf(&mut rng, path, "pk_tree_5")?;

    setup_pk_tree_node_mnt4(&mut rng, path, "pk_tree_4", 4)?;

    setup_pk_tree_node_mnt6(&mut rng, path, "pk_tree_3", 3)?;

    setup_pk_tree_node_mnt4(&mut rng, path, "pk_tree_2", 2)?;

    setup_pk_tree_node_mnt6(&mut rng, path, "pk_tree_1", 1)?;

    setup_pk_tree_node_mnt4(&mut rng, path, "pk_tree_0", 0)?;

    setup_macro_block(&mut rng, path)?;

    setup_macro_block_wrapper(&mut rng, path)?;

    setup_merger(&mut rng, path)?;

    setup_merger_wrapper(&mut rng, path)?;

    save_metadata_to_file(path, network_id)?;

    Ok(())
}

pub fn load_verifying_data(path: &Path) -> Result<VerifyingData, NanoZKPError> {
    let metadata = fs::read(path.join("meta_data.bin"))?;
    let metadata = VerifyingKeyMetadata::deserialize_from_vec(&metadata)?;

    Ok(VerifyingData {
        merger_wrapper_vk: load_key(&path.join("verifying_keys"), "merger_wrapper")?,
        keys_commitment: metadata.vks_commitment(),
    })
}

fn load_key<E: Pairing>(dir_path: &Path, file_name: &str) -> Result<VerifyingKey<E>, NanoZKPError> {
    let mut file = File::open(dir_path.join(format!("{file_name}.bin")))?;
    Ok(VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?)
}

pub fn save_metadata_to_file(path: &Path, network_id: NetworkId) -> Result<(), NanoZKPError> {
    let network_info = NetworkInfo::from_network_id(network_id);
    let genesis_block = network_info.genesis_block().unwrap_macro();
    let keys = load_keys(path)?;

    let meta_data = VerifyingKeyMetadata::new(genesis_block.hash(), keys.commitment());

    Ok(meta_data.save_to_file(path)?)
}

pub fn load_keys(dir_path: &Path) -> Result<VerifyingKeys, NanoZKPError> {
    let verifying_keys = dir_path.join("verifying_keys");
    let merger_wrapper = load_key(&verifying_keys, "merger_wrapper")?;
    let merger = load_key(&verifying_keys, "merger")?;
    let macro_block_wrapper = load_key(&verifying_keys, "macro_block_wrapper")?;
    let macro_block = load_key(&verifying_keys, "macro_block")?;
    let pk_tree0 = load_key(&verifying_keys, "pk_tree_0")?;
    let pk_tree1 = load_key(&verifying_keys, "pk_tree_1")?;
    let pk_tree2 = load_key(&verifying_keys, "pk_tree_2")?;
    let pk_tree3 = load_key(&verifying_keys, "pk_tree_3")?;
    let pk_tree4 = load_key(&verifying_keys, "pk_tree_4")?;
    let pk_tree_leaf = load_key(&verifying_keys, "pk_tree_5")?;
    Ok(VerifyingKeys::new(
        merger_wrapper,
        merger,
        macro_block_wrapper,
        macro_block,
        pk_tree0,
        pk_tree1,
        pk_tree2,
        pk_tree3,
        pk_tree4,
        pk_tree_leaf,
    ))
}

pub fn all_files_created(path: &Path, prover_active: bool) -> bool {
    let verifying_keys = path.join("verifying_keys");
    let proving_keys = path.join("proving_keys");

    if prover_active {
        for i in 0..5 {
            if !verifying_keys.join(format!("pk_tree_{i}.bin")).exists()
                || !proving_keys.join(format!("pk_tree_{i}.bin")).exists()
            {
                return false;
            }
        }
    }

    verifying_keys.join("merger_wrapper.bin").exists()
        && (!prover_active
            || (verifying_keys.join("macro_block.bin").exists()
                && verifying_keys.join("macro_block_wrapper.bin").exists()
                && verifying_keys.join("merger.bin").exists()
                && proving_keys.join("merger_wrapper.bin").exists()
                && proving_keys.join("macro_block.bin").exists()
                && proving_keys.join("macro_block_wrapper.bin").exists()
                && proving_keys.join("merger.bin").exists()))
}

fn setup_pk_tree_leaf<R: Rng + CryptoRng>(
    rng: &mut R,
    dir_path: &Path,
    name: &str,
) -> Result<(), NanoZKPError> {
    if keys_exist(name, dir_path) {
        return Ok(());
    }

    let circuit: LeafMNT6 = rng.gen();

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &vk, name, dir_path)
}

fn setup_pk_tree_node_mnt4<R: Rng + CryptoRng>(
    rng: &mut R,
    dir_path: &Path,
    name: &str,
    tree_level: usize,
) -> Result<(), NanoZKPError> {
    if keys_exist(name, dir_path) {
        return Ok(());
    }

    // Create parameters for our circuit
    let circuit = NodeMNT4::rand(tree_level, rng);

    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &vk, name, dir_path)
}

fn setup_pk_tree_node_mnt6<R: Rng + CryptoRng>(
    rng: &mut R,
    dir_path: &Path,
    name: &str,
    tree_level: usize,
) -> Result<(), NanoZKPError> {
    if keys_exist(name, dir_path) {
        return Ok(());
    }

    // Create parameters for our circuit
    let circuit = NodeMNT6::rand(tree_level, rng);

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &vk, name, dir_path)
}

fn setup_macro_block<R: Rng + CryptoRng>(rng: &mut R, path: &Path) -> Result<(), NanoZKPError> {
    if keys_exist("macro_block", path) {
        return Ok(());
    }

    // Create parameters for our circuit
    let circuit = MacroBlockCircuit::rand(rng);

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;
    assert_eq!(vk.gamma_abc_g1.len(), MacroBlockCircuit::NUM_INPUTS + 1);

    // Save keys to file.
    keys_to_file(&pk, &vk, "macro_block", path)
}

fn setup_macro_block_wrapper<R: Rng + CryptoRng>(
    rng: &mut R,
    path: &Path,
) -> Result<(), NanoZKPError> {
    if keys_exist("macro_block_wrapper", path) {
        return Ok(());
    }

    // Load the verifying key from file.
    // Create parameters for our circuit
    let circuit = MacroBlockWrapperCircuit::rand(rng);

    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng)?;
    assert_eq!(
        vk.gamma_abc_g1.len(),
        MacroBlockWrapperCircuit::NUM_INPUTS + 1
    );

    // Save keys to file.
    keys_to_file(&pk, &vk, "macro_block_wrapper", path)
}

fn setup_merger<R: Rng + CryptoRng>(rng: &mut R, path: &Path) -> Result<(), NanoZKPError> {
    if keys_exist("merger", path) {
        return Ok(());
    }

    let circuit = MergerCircuit::rand(rng);

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;
    assert_eq!(vk.gamma_abc_g1.len(), MergerCircuit::NUM_INPUTS + 1);

    // Save keys to file.
    keys_to_file(&pk, &vk, "merger", path)
}

fn setup_merger_wrapper<R: Rng + CryptoRng>(rng: &mut R, path: &Path) -> Result<(), NanoZKPError> {
    if keys_exist("merger_wrapper", path) {
        return Ok(());
    }

    // Create parameters for our circuit
    let circuit = MergerWrapperCircuit::rand(rng);

    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng)?;
    assert_eq!(vk.gamma_abc_g1.len(), MergerWrapperCircuit::NUM_INPUTS + 1);

    // Save keys to file.
    keys_to_file(&pk, &vk, "merger_wrapper", path)
}

pub(crate) fn keys_exist(name: &str, path: &Path) -> bool {
    let verifying_key = path.join("verifying_keys").join(format!("{name}.bin"));
    let proving_key = path.join("proving_keys").join(format!("{name}.bin"));

    proving_key.exists() && verifying_key.exists()
}

pub(crate) fn keys_to_file<T: Pairing>(
    pk: &ProvingKey<T>,
    vk: &VerifyingKey<T>,
    name: &str,
    path: &Path,
) -> Result<(), NanoZKPError> {
    let verifying_keys = path.join("verifying_keys");
    let proving_keys = path.join("proving_keys");

    // Save proving key to file.
    if !proving_keys.is_dir() {
        DirBuilder::new().recursive(true).create(&proving_keys)?;
    }

    let mut file = File::create(proving_keys.join(format!("{name}.bin")))?;
    pk.serialize_uncompressed(&mut file)?;
    file.sync_all()?;

    // Save verifying key to file.
    if !verifying_keys.is_dir() {
        DirBuilder::new().recursive(true).create(&verifying_keys)?;
    }

    let mut file = File::create(verifying_keys.join(format!("{name}.bin")))?;
    vk.serialize_uncompressed(&mut file)?;
    file.sync_all()?;

    Ok(())
}
