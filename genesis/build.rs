use std::{env, fmt::Write, fs, path::Path};

use nimiq_database::volatile::VolatileDatabase;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_hash::Blake2bHash;

fn write_genesis_rs(directory: &Path, name: &str, genesis_hash: &Blake2bHash) {
    let hash = {
        let mut hash = String::new();
        write!(&mut hash, "0x{:02x}", genesis_hash.0[0]).unwrap();
        for &byte in &genesis_hash.0[1..] {
            write!(&mut hash, ", 0x{:02x}", byte).unwrap();
        }
        hash
    };

    let genesis_rs = format!(
        r#"GenesisData {{
            block: include_bytes!(concat!(env!("OUT_DIR"), "/genesis/{name}/block.dat")),
            hash: Blake2bHash([{hash}]),
            accounts: include_bytes!(concat!(env!("OUT_DIR"), "/genesis/{name}/accounts.dat")),
    }}"#,
    );
    log::debug!("Writing genesis source code: {}", &genesis_rs);
    fs::write(directory.join("genesis.rs"), genesis_rs.as_bytes()).unwrap();
}

fn generate_albatross(name: &str, out_dir: &Path, src_dir: &Path) {
    log::info!("Generating Albatross genesis config: {}", name);

    let directory = out_dir.join(name);
    fs::create_dir_all(&directory).unwrap();

    let genesis_config = src_dir.join(format!("{name}.toml"));
    log::info!("genesis source file: {}", genesis_config.display());

    let env = VolatileDatabase::new(20).expect("Could not open a volatile database");
    let builder = GenesisBuilder::from_config_file(genesis_config).unwrap();
    let genesis_hash = builder.write_to_files(env, &directory).unwrap();
    write_genesis_rs(&directory, name, &genesis_hash);
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let out_dir = Path::new(&env::var("OUT_DIR").unwrap()).join("genesis");
    let src_dir = Path::new("src").join("genesis");

    println!("Taking genesis config files from: {}", src_dir.display());
    println!("Writing genesis data to: {}", out_dir.display());
    generate_albatross("dev-albatross", &out_dir, &src_dir);
    generate_albatross("test-albatross", &out_dir, &src_dir);
    generate_albatross("unit-albatross", &out_dir, &src_dir);
    generate_albatross("main-albatross", &out_dir, &src_dir);
}
