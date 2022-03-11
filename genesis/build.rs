use std::env;
use std::fs;
use std::path::Path;

use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_hash::Blake2bHash;

fn write_genesis_rs(directory: &Path, name: &str, genesis_hash: &Blake2bHash) {
    let genesis_rs = format!(
        r#"GenesisData {{
            block: include_bytes!(concat!(env!("OUT_DIR"), "/genesis/{}/block.dat")),
            hash: "{}".into(),
            accounts: include_bytes!(concat!(env!("OUT_DIR"), "/genesis/{}/accounts.dat")),
    }}"#,
        name, genesis_hash, name,
    );
    log::debug!("Writing genesis source code: {}", &genesis_rs);
    fs::write(directory.join("genesis.rs"), genesis_rs.as_bytes()).unwrap();
}

fn generate_albatross(name: &str, out_dir: &Path, src_dir: &Path) {
    log::info!("Generating Albatross genesis config: {}", name);

    let directory = out_dir.join(name);
    fs::create_dir_all(&directory).unwrap();

    let genesis_config = src_dir.join(format!("{}.toml", name));
    log::info!("genesis source file: {}", genesis_config.display());

    let mut builder = GenesisBuilder::new();
    let env = VolatileEnvironment::new(10).expect("Could not open a volatile database");
    builder.with_config_file(genesis_config).unwrap();
    let genesis_hash = builder.write_to_files(env, &directory).unwrap();
    write_genesis_rs(&directory, name, &genesis_hash);
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let out_dir = Path::new(&env::var("OUT_DIR").unwrap()).join("genesis");
    let src_dir = Path::new("src").join("genesis");

    log::info!("Taking genesis config files from: {}", src_dir.display());
    log::info!("Writing genesis data to: {}", out_dir.display());
    log::error!(
        "DevNet override {:?}",
        env::var("NIMIQ_OVERRIDE_DEVNET_CONFIG")
    );

    generate_albatross("dev-albatross", &out_dir, &src_dir);
    generate_albatross("unit-albatross", &out_dir, &src_dir);
}
