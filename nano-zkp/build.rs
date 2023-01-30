use std::{env, path::Path};

use nimiq_compile_zkp_setup::verifying_keys_setup;
use nimiq_nano_primitives::{NanoZKPError, KEYS_PATH};

fn main() -> Result<(), NanoZKPError> {
    let keys_dest_path = Path::new(&env::var_os("OUT_DIR").unwrap()).join("verifying_keys.data");

    let keys_source_path = Path::new(KEYS_PATH);

    verifying_keys_setup(keys_source_path, &keys_dest_path)?;

    println!("cargo:rerun-if-changed=build.rs");

    Ok(())
}
