use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use nimiq_build_tools::genesis::GenesisBuilder;
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

fn generate_albatross(
    name: &str,
    out_dir: &Path,
    src_dir: &Path,
    config_override: Option<PathBuf>,
) {
    log::info!("Generating Albatross genesis config: {}", name);

    let directory = out_dir.join(name);
    fs::create_dir_all(&directory).unwrap();

    let genesis_config = if let Some(config_override) = config_override {
        config_override
    } else {
        src_dir.join(format!("{}.toml", name))
    };
    log::info!("genesis source file: {}", genesis_config.display());

    let mut builder = GenesisBuilder::new();
    builder.with_config_file(genesis_config).unwrap();
    let genesis_hash = builder.write_to_files(&directory).unwrap();
    write_genesis_rs(&directory, name, &genesis_hash);
}

fn main() {
    pretty_env_logger::init();

    let out_dir = Path::new(&env::var("OUT_DIR").unwrap()).join("genesis");
    let src_dir = Path::new("src").join("genesis");
    let devnet_override = env::var("NIMIQ_OVERRIDE_DEVNET_CONFIG")
        .ok()
        .map(PathBuf::from);

    log::info!("Taking genesis config files from: {}", src_dir.display());
    log::info!("Writing genesis data to: {}", out_dir.display());
    log::error!(
        "DevNet override {:?}",
        env::var("NIMIQ_OVERRIDE_DEVNET_CONFIG")
    );
    if let Some(devnet_override) = &devnet_override {
        log::info!(
            "Using override for Albatross DevNet config: {}",
            devnet_override.display()
        );
    }

    generate_albatross("dev-albatross", &out_dir, &src_dir, devnet_override);
    generate_albatross("unit-albatross", &out_dir, &src_dir, None);
}
