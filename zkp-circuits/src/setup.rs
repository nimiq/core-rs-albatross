use std::fs::{DirBuilder, File};
use std::path::Path;

use ark_crypto_primitives::snark::CircuitSpecificSetupSNARK;
use ark_ec::{mnt6::MNT6, pairing::Pairing};
use ark_groth16::{Groth16, ProvingKey, VerifyingKey};
use ark_mnt4_753::MNT4_753;
use ark_mnt6_753::{Config, MNT6_753};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use rand::{CryptoRng, Rng};

use nimiq_zkp_primitives::NanoZKPError;

use crate::{
    circuits::mnt4::{
        MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as NodeMNT4,
    },
    circuits::mnt6::{
        MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit as LeafMNT6,
        PKTreeNodeCircuit as NodeMNT6,
    },
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
    prover_active: bool,
) -> Result<(), NanoZKPError> {
    if all_files_created(path, prover_active) {
        return Ok(());
    }

    setup_pk_tree_leaf(&mut rng, path, "pk_tree_5")?;

    setup_pk_tree_node_mnt4(&mut rng, path, "pk_tree_5", "pk_tree_4", 4)?;

    setup_pk_tree_node_mnt6(&mut rng, path, "pk_tree_4", "pk_tree_3", 3)?;

    setup_pk_tree_node_mnt4(&mut rng, path, "pk_tree_3", "pk_tree_2", 2)?;

    setup_pk_tree_node_mnt6(&mut rng, path, "pk_tree_2", "pk_tree_1", 1)?;

    setup_pk_tree_node_mnt4(&mut rng, path, "pk_tree_1", "pk_tree_0", 0)?;

    setup_macro_block(&mut rng, path)?;

    setup_macro_block_wrapper(&mut rng, path)?;

    setup_merger(&mut rng, path)?;

    setup_merger_wrapper(&mut rng, path)?;

    Ok(())
}

pub fn load_verifying_key_from_file(
    path: &Path,
) -> Result<VerifyingKey<MNT6<Config>>, NanoZKPError> {
    // Loads the verifying key from the preexisting file.
    // Note: We only use the merger_wrapper for key verification purposes.
    let mut file = File::open(path.join("verifying_keys").join("merger_wrapper.bin"))?;
    let vk: VerifyingKey<MNT6<Config>> =
        VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    Ok(vk)
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
                && proving_keys.join("macro_block_wrapper.bin").exists()
                && proving_keys.join("merger.bin").exists()
                && proving_keys.join("merger_wrapper.bin").exists()))
}

fn setup_pk_tree_leaf<R: Rng + CryptoRng>(
    rng: &mut R,
    dir_path: &Path,
    name: &str,
) -> Result<(), NanoZKPError> {
    let circuit: LeafMNT6 = rng.gen();

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &vk, name, dir_path)
}

fn setup_pk_tree_node_mnt4<R: Rng + CryptoRng>(
    rng: &mut R,
    dir_path: &Path,
    vk_file: &str,
    name: &str,
    tree_level: usize,
) -> Result<(), NanoZKPError> {
    // Load the verifying key from file.
    let mut file = File::open(
        dir_path
            .join("verifying_keys")
            .join(format!("{vk_file}.bin")),
    )?;

    let vk_child = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Create parameters for our circuit
    let circuit = NodeMNT4::rand(tree_level, vk_child, rng);

    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &vk, name, dir_path)
}

fn setup_pk_tree_node_mnt6<R: Rng + CryptoRng>(
    rng: &mut R,
    dir_path: &Path,
    vk_file: &str,
    name: &str,
    tree_level: usize,
) -> Result<(), NanoZKPError> {
    // Load the verifying key from file.
    let mut file = File::open(
        dir_path
            .join("verifying_keys")
            .join(format!("{vk_file}.bin")),
    )?;

    let vk_child = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Create parameters for our circuit
    let circuit = NodeMNT6::rand(tree_level, vk_child, rng);

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &vk, name, dir_path)
}

fn setup_macro_block<R: Rng + CryptoRng>(rng: &mut R, path: &Path) -> Result<(), NanoZKPError> {
    // Load the verifying key from file.
    let mut file = File::open(path.join("verifying_keys").join("pk_tree_0.bin"))?;

    let vk_pk_tree = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Create parameters for our circuit
    let circuit = MacroBlockCircuit::rand(vk_pk_tree, rng);

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &vk, "macro_block", path)
}

fn setup_macro_block_wrapper<R: Rng + CryptoRng>(
    rng: &mut R,
    path: &Path,
) -> Result<(), NanoZKPError> {
    // Load the verifying key from file.
    let mut file = File::open(path.join("verifying_keys").join("macro_block.bin"))?;

    let vk_macro_block = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Create parameters for our circuit
    let circuit = MacroBlockWrapperCircuit::rand(vk_macro_block, rng);

    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &vk, "macro_block_wrapper", path)
}

fn setup_merger<R: Rng + CryptoRng>(rng: &mut R, path: &Path) -> Result<(), NanoZKPError> {
    // Load the verifying key from file.
    let mut file = File::open(path.join("verifying_keys").join("macro_block_wrapper.bin"))?;

    let vk_macro_block_wrapper = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    let circuit = MergerCircuit::rand(vk_macro_block_wrapper, rng);

    let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &vk, "merger", path)
}

fn setup_merger_wrapper<R: Rng + CryptoRng>(rng: &mut R, path: &Path) -> Result<(), NanoZKPError> {
    // Load the verifying key from file.
    let mut file = File::open(path.join("verifying_keys").join("merger.bin"))?;

    let vk_merger = VerifyingKey::deserialize_uncompressed_unchecked(&mut file)?;

    // Create parameters for our circuit
    let circuit = MergerWrapperCircuit::rand(vk_merger, rng);

    let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &vk, "merger_wrapper", path)
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
