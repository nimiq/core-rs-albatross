use ark_ec::mnt6::MNT6;
use ark_groth16::VerifyingKey;
use ark_mnt6_753::Parameters;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use std::{env, fs::File, path::Path};

use nimiq_nano_primitives::{
    setup::{all_files_created, setup},
    NanoZKPError, SEED,
};

pub fn verifying_keys_setup(source_path: &Path, dest_path: &Path) -> Result<(), NanoZKPError> {
    let is_prover_feature = env::var_os("CARGO_FEATURE_ZKP_PROVER")
        .map_or(false, |var| var.into_string().unwrap() == 1.to_string());

    // Generates the verifying keys if they don't exist yet.
    if !all_files_created(source_path, false) {
        eprintln!(
            "Missing the verifying keys, so we will generate both proving and verifying keys now."
        );
        eprintln!("This process takes approximately 1 hour.");
        eprintln!(
            "Alternatively you can provide the proving keys by placing them on {:?}",
            source_path
        );

        setup(ChaCha20Rng::from_seed(SEED), source_path, is_prover_feature)?;
    } else {
        // If the proving keys are missing, we will just print a warning since the keys will be generated at run time.
        if !all_files_created(source_path, is_prover_feature) {
            eprintln!("The proving keys were not provided! To run a prover node, at run time, both verifying and proving keys will be generated.");
            eprintln!(
                "Alternatively you can provide the proving keys by placing them on {:?}",
                source_path
            );
        }
    }

    // Loads the verifying key from the preexisting file.
    // Note: We only use the merger_wrapper for key verification purposes.
    let mut file = File::open(
        source_path
            .join("verifying_keys")
            .join("merger_wrapper.bin"),
    )?;
    let vk: VerifyingKey<MNT6<Parameters>> = VerifyingKey::deserialize_unchecked(&mut file)?;

    // Creates the destination files to be included in the binary and writes the key to them.
    let mut dest_file = File::create(dest_path)?;
    vk.serialize_unchecked(&mut dest_file)?;
    dest_file.sync_all()?;

    Ok(())
}
