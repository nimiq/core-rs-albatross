use nimiq_pedersen_generators::{generate_pedersen_generators, SerializingError};
use std::env;
use std::path::Path;

fn main() -> Result<(), SerializingError> {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("pedersen_generator_powers.data");

    generate_pedersen_generators(dest_path)?;

    println!("cargo:rerun-if-changed=build.rs");
    Ok(())
}
