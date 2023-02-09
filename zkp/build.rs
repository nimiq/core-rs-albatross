use std::{env, fs::File, path::Path};

use ark_serialize::CanonicalSerialize;

use nimiq_zkp_circuits::setup::{all_files_created, load_verifying_key_from_file};
use nimiq_zkp_primitives::NanoZKPError;

pub fn verifying_keys_setup(source_path: &Path, dest_path: &Path) -> Result<(), NanoZKPError> {
    // Generates the verifying keys if they don't exist yet.
    if !all_files_created(source_path, false) {
        eprintln!(
            "Missing the verifying keys! You can run for generating the proving and verifying keys now."
        );
        eprintln!(
            "Alternatively you can provide the proving keys by placing them on {:?}",
            source_path.canonicalize()
        );
        panic!("Missing verifying keys!");
    }

    // Loads the verifying key from the preexisting file.
    // Note: We only use the merger_wrapper for key verification purposes.
    let vk = load_verifying_key_from_file(source_path)?;

    // Creates the destination files to be included in the binary and writes the key to them.
    let mut dest_file = File::create(dest_path)?;
    vk.serialize_uncompressed(&mut dest_file)?;
    dest_file.sync_all()?;

    Ok(())
}

fn main() -> Result<(), NanoZKPError> {
    let keys_dest_path = Path::new(&env::var_os("OUT_DIR").unwrap()).join("verifying_key.data");
    let keys_source_path = Path::new("../.zkp");

    verifying_keys_setup(keys_source_path, &keys_dest_path)?;

    println!("cargo:rerun-if-changed=build.rs");

    Ok(())
}
