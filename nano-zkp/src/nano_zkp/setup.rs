use std::fs::{DirBuilder, File};
use std::path::Path;

use ark_crypto_primitives::CircuitSpecificSetupSNARK;
use ark_ec::{PairingEngine, ProjectiveCurve};
use ark_groth16::{Groth16, Proof, ProvingKey, VerifyingKey};
use ark_mnt4_753::{Fr as MNT4Fr, G1Projective as G1MNT4, G2Projective as G2MNT4, MNT4_753};
use ark_mnt6_753::{Fr as MNT6Fr, G1Projective as G1MNT6, G2Projective as G2MNT6, MNT6_753};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::UniformRand;
use rand::{CryptoRng, Rng};

use nimiq_bls::utils::bytes_to_bits;
use nimiq_nano_primitives::{MacroBlock, PK_TREE_BREADTH, PK_TREE_DEPTH};
use nimiq_primitives::policy::Policy;

use crate::circuits::mnt4::{
    MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit as LeafMNT4, PKTreeNodeCircuit as NodeMNT4,
};
use crate::circuits::mnt6::{
    MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as NodeMNT6,
};
use crate::{NanoZKP, NanoZKPError};

impl NanoZKP {
    /// This function generates the parameters (proving and verifying keys) for the entire nano sync
    /// program. It does this by generating the parameters for each circuit, "from bottom to top". The
    /// order is absolutely necessary because each circuit needs a verifying key from the circuit "below"
    /// it. Note that the parameter generation can take longer than one hour, even two on some computers.
    pub fn setup<R: Rng + CryptoRng>(
        mut rng: R,
        path: &Path,
        prover_active: bool,
    ) -> Result<(), NanoZKPError> {
        if NanoZKP::all_files_created(path, prover_active) {
            return Ok(());
        }

        NanoZKP::setup_pk_tree_leaf(&mut rng, path, "pk_tree_5")?;

        NanoZKP::setup_pk_tree_node_mnt6(&mut rng, path, "pk_tree_5", "pk_tree_4", 4)?;

        NanoZKP::setup_pk_tree_node_mnt4(&mut rng, path, "pk_tree_4", "pk_tree_3", 3)?;

        NanoZKP::setup_pk_tree_node_mnt6(&mut rng, path, "pk_tree_3", "pk_tree_2", 2)?;

        NanoZKP::setup_pk_tree_node_mnt4(&mut rng, path, "pk_tree_2", "pk_tree_1", 1)?;

        NanoZKP::setup_pk_tree_node_mnt6(&mut rng, path, "pk_tree_1", "pk_tree_0", 0)?;

        NanoZKP::setup_macro_block(&mut rng, path)?;

        NanoZKP::setup_macro_block_wrapper(&mut rng, path)?;

        NanoZKP::setup_merger(&mut rng, path)?;

        NanoZKP::setup_merger_wrapper(&mut rng, path)?;

        Ok(())
    }

    fn all_files_created(path: &Path, prover_active: bool) -> bool {
        let verifying_keys = path.join("verifying_keys");
        let proving_keys = path.join("proving_keys");

        for i in 0..5 {
            if !verifying_keys.join(&format!("pk_tree_{}.bin", i)).exists()
                || (prover_active && !proving_keys.join(&format!("pk_tree_{}.bin", i)).exists())
            {
                return false;
            }
        }

        verifying_keys.join("macro_block.bin").exists()
            && verifying_keys.join("macro_block_wrapper.bin").exists()
            && verifying_keys.join("merger.bin").exists()
            && verifying_keys.join("merger_wrapper.bin").exists()
            && (!prover_active
                || (proving_keys.join("merger_wrapper.bin").exists()
                    && proving_keys.join("macro_block_wrapper.bin").exists()
                    && proving_keys.join("merger.bin").exists()
                    && proving_keys.join("merger_wrapper.bin").exists()))
    }

    fn setup_pk_tree_leaf<R: Rng + CryptoRng>(
        rng: &mut R,
        dir_path: &Path,
        name: &str,
    ) -> Result<(), NanoZKPError> {
        // Create dummy inputs.
        let pks = vec![G2MNT6::rand(rng); Policy::SLOTS as usize / PK_TREE_BREADTH];

        let pk_tree_nodes = vec![G1MNT6::rand(rng); PK_TREE_DEPTH];

        let pk_tree_root = vec![MNT4Fr::rand(rng); 2];

        let agg_pk_commitment = vec![MNT4Fr::rand(rng); 2];

        let signer_bitmap = MNT4Fr::rand(rng);

        let path = MNT4Fr::rand(rng);

        // Create parameters for our circuit
        let circuit = LeafMNT4::new(
            pks,
            pk_tree_nodes,
            pk_tree_root,
            agg_pk_commitment,
            signer_bitmap,
            path,
        );

        let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;

        // Save keys to file.
        NanoZKP::keys_to_file(pk, vk, name, dir_path)
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
                .join(format!("{}.bin", vk_file)),
        )?;

        let vk_child = VerifyingKey::deserialize_unchecked(&mut file)?;

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
        let circuit = NodeMNT6::new(
            tree_level,
            vk_child,
            left_proof,
            right_proof,
            pk_tree_root,
            left_agg_pk_commitment,
            right_agg_pk_commitment,
            signer_bitmap,
            path,
        );

        let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng)?;

        // Save keys to file.
        NanoZKP::keys_to_file(pk, vk, name, dir_path)
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
                .join(format!("{}.bin", vk_file)),
        )?;

        let vk_child = VerifyingKey::deserialize_unchecked(&mut file)?;

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
        let circuit = NodeMNT4::new(
            tree_level,
            vk_child,
            left_proof,
            right_proof,
            agg_pk_chunks,
            pk_tree_root,
            agg_pk_commitment,
            signer_bitmap,
            path,
        );

        let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;

        // Save keys to file.
        NanoZKP::keys_to_file(pk, vk, name, dir_path)
    }

    fn setup_macro_block<R: Rng + CryptoRng>(rng: &mut R, path: &Path) -> Result<(), NanoZKPError> {
        // Load the verifying key from file.
        let mut file = File::open(path.join("verifying_keys").join("pk_tree_0.bin"))?;

        let vk_pk_tree = VerifyingKey::deserialize_unchecked(&mut file)?;

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

        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        let initial_header_hash = bytes_to_bits(&bytes);

        let block_number = u32::rand(rng);

        let round_number = u32::rand(rng);

        let mut bytes = [0u8; 95];
        rng.fill_bytes(&mut bytes);
        let final_pk_tree_root = bytes_to_bits(&bytes);

        let initial_state_commitment = vec![MNT4Fr::rand(rng); 2];

        let final_state_commitment = vec![MNT4Fr::rand(rng); 2];

        let mut header_hash = [0u8; 32];
        rng.fill_bytes(&mut header_hash);

        let signature = G1MNT6::rand(rng);

        let mut bytes = [0u8; Policy::SLOTS as usize / 8];
        rng.fill_bytes(&mut bytes);
        let signer_bitmap = bytes_to_bits(&bytes);

        let block = MacroBlock {
            block_number,
            round_number,
            header_hash,
            signature,
            signer_bitmap,
        };

        // Create parameters for our circuit
        let circuit = MacroBlockCircuit::new(
            vk_pk_tree,
            agg_pk_chunks,
            proof,
            initial_pk_tree_root,
            initial_header_hash,
            final_pk_tree_root,
            block,
            initial_state_commitment,
            final_state_commitment,
        );

        let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;

        // Save keys to file.
        NanoZKP::keys_to_file(pk, vk, "macro_block", path)
    }

    fn setup_macro_block_wrapper<R: Rng + CryptoRng>(
        rng: &mut R,
        path: &Path,
    ) -> Result<(), NanoZKPError> {
        // Load the verifying key from file.
        let mut file = File::open(path.join("verifying_keys").join("macro_block.bin"))?;

        let vk_macro_block = VerifyingKey::deserialize_unchecked(&mut file)?;

        // Create dummy inputs.
        let proof = Proof {
            a: G1MNT4::rand(rng).into_affine(),
            b: G2MNT4::rand(rng).into_affine(),
            c: G1MNT4::rand(rng).into_affine(),
        };

        let initial_state_commitment = vec![MNT6Fr::rand(rng); 2];

        let final_state_commitment = vec![MNT6Fr::rand(rng); 2];

        // Create parameters for our circuit
        let circuit = MacroBlockWrapperCircuit::new(
            vk_macro_block,
            proof,
            initial_state_commitment,
            final_state_commitment,
        );

        let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng)?;

        // Save keys to file.
        NanoZKP::keys_to_file(pk, vk, "macro_block_wrapper", path)
    }

    fn setup_merger<R: Rng + CryptoRng>(rng: &mut R, path: &Path) -> Result<(), NanoZKPError> {
        // Load the verifying key from file.
        let mut file = File::open(path.join("verifying_keys").join("macro_block_wrapper.bin"))?;

        let vk_macro_block_wrapper = VerifyingKey::deserialize_unchecked(&mut file)?;

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

        let (pk, vk) = Groth16::<MNT4_753>::setup(circuit, rng)?;

        // Save keys to file.
        NanoZKP::keys_to_file(pk, vk, "merger", path)
    }

    fn setup_merger_wrapper<R: Rng + CryptoRng>(
        rng: &mut R,
        path: &Path,
    ) -> Result<(), NanoZKPError> {
        // Load the verifying key from file.
        let mut file = File::open(path.join("verifying_keys").join("merger.bin"))?;

        let vk_merger = VerifyingKey::deserialize_unchecked(&mut file)?;

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
        let circuit = MergerWrapperCircuit::new(
            vk_merger,
            proof,
            initial_state_commitment,
            final_state_commitment,
            vk_commitment,
        );

        let (pk, vk) = Groth16::<MNT6_753>::setup(circuit, rng)?;

        // Save keys to file.
        NanoZKP::keys_to_file(pk, vk, "merger_wrapper", path)
    }

    fn keys_to_file<T: PairingEngine>(
        pk: ProvingKey<T>,
        vk: VerifyingKey<T>,
        name: &str,
        path: &Path,
    ) -> Result<(), NanoZKPError> {
        let verifying_keys = path.join("verifying_keys");
        let proving_keys = path.join("proving_keys");

        // Save proving key to file.
        if !proving_keys.is_dir() {
            DirBuilder::new().create(&proving_keys)?;
        }

        let mut file = File::create(proving_keys.join(format!("{}.bin", name)))?;

        pk.serialize_unchecked(&mut file)?;

        file.sync_all()?;

        // Save verifying key to file.
        if !verifying_keys.is_dir() {
            DirBuilder::new().create(&verifying_keys)?;
        }

        let mut file = File::create(verifying_keys.join(format!("{}.bin", name)))?;

        vk.serialize_unchecked(&mut file)?;

        file.sync_all()?;

        Ok(())
    }
}
