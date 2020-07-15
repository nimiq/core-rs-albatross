#[macro_use]
extern crate log;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use log::Level;

use nimiq_build_tools::genesis::albatross::GenesisBuilder;
use nimiq_build_tools::genesis::powchain::PowChainGenesis;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;

fn write_genesis_rs(
    directory: &PathBuf,
    name: &str,
    genesis_hash: &Blake2bHash,
    validator_registry: Option<Address>,
) {
    let validator_registry_str;
    if let Some(address) = validator_registry {
        validator_registry_str = format!("Some(\"{}\".into())", address);
    } else {
        validator_registry_str = "None".to_string();
    }
    let genesis_rs = format!(
        r#"GenesisData {{
            block: include_bytes!(concat!(env!("OUT_DIR"), "/genesis/{}/block.dat")),
            hash: "{}".into(),
            accounts: include_bytes!(concat!(env!("OUT_DIR"), "/genesis/{}/accounts.dat")),
            validator_registry: {},
    }}"#,
        name, genesis_hash, name, validator_registry_str
    );
    debug!("Writing genesis source code: {}", &genesis_rs);
    fs::write(directory.join("genesis.rs"), genesis_rs.as_bytes()).unwrap();
}

fn generate_powchain(name: &str, out_dir: &PathBuf, powchain: PowChainGenesis) {
    let directory = out_dir.join(name);
    fs::create_dir_all(&directory).unwrap();
    let genesis_hash = powchain.generate_genesis_hash().unwrap();

    powchain.write_to_files(&directory).unwrap();
    write_genesis_rs(&directory, name, &genesis_hash, None);
}

fn generate_albatross(
    name: &str,
    out_dir: &PathBuf,
    src_dir: &PathBuf,
    config_override: Option<PathBuf>,
) {
    info!("Generating Albatross genesis config: {}", name);

    let directory = out_dir.join(name);
    fs::create_dir_all(&directory).unwrap();

    let genesis_config = if let Some(config_override) = config_override {
        config_override
    } else {
        src_dir.join(format!("{}.toml", name))
    };
    info!("genesis source file: {}", genesis_config.display());

    let mut builder = GenesisBuilder::default();
    builder.with_config_file(genesis_config).unwrap();
    let staking_contract_address = builder
        .staking_contract_address
        .clone()
        .expect("Missing staking contract address");
    let genesis_hash = builder.write_to_files(&directory).unwrap();
    write_genesis_rs(
        &directory,
        name,
        &genesis_hash,
        Some(staking_contract_address),
    );
}

fn main() {
    //setup_panic!();
    simple_logger::init_with_level(Level::Debug)
        .unwrap_or_else(|e| eprintln!("Failed to initialize logging: {}", e));

    let out_dir = Path::new(&env::var("OUT_DIR").unwrap()).join("genesis");
    let src_dir = Path::new("src").join("genesis");
    let devnet_override = env::var("NIMIQ_OVERRIDE_DEVNET_CONFIG")
        .ok()
        .map(|devnet_override| PathBuf::from(devnet_override));

    info!("Taking genesis config files from: {}", src_dir.display());
    info!("Writing genesis data to: {}", out_dir.display());
    error!(
        "DevNet override {:?}",
        env::var("NIMIQ_OVERRIDE_DEVNET_CONFIG")
    );
    if let Some(devnet_override) = &devnet_override {
        info!(
            "Using override for Albatross DevNet config: {}",
            devnet_override.display()
        );
    }

    generate_powchain("main-powchain", &out_dir, PowChainGenesis::main());
    generate_powchain("test-powchain", &out_dir, PowChainGenesis::test());
    generate_powchain("dev-powchain", &out_dir, PowChainGenesis::dev());
    generate_albatross("dev-albatross", &out_dir, &src_dir, devnet_override);
    generate_albatross("unit-albatross", &out_dir, &src_dir, None);
}
