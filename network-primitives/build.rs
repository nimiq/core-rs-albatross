#[macro_use]
extern crate human_panic;
#[macro_use]
extern crate log;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use log::Level;

use nimiq_build_tools::genesis::albatross::GenesisBuilder;
use nimiq_build_tools::genesis::powchain::PowChainGenesis;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;

fn write_genesis_rs(directory: &PathBuf, name: &str, genesis_hash: &Blake2bHash, validator_registry: Option<Address>) {
    let validator_registry_str;
    if let Some(address) = validator_registry {
        validator_registry_str = format!("Some(\"{}\".into())", address);
    } else {
        validator_registry_str = "None".to_string();
    }
    let genesis_rs = format!(r#"GenesisData {{
            block: include_bytes!(concat!(env!("OUT_DIR"), "/genesis/{}/block.dat")),
            hash: "{}".into(),
            accounts: include_bytes!(concat!(env!("OUT_DIR"), "/genesis/{}/accounts.dat")),
            validator_registry: {},
    }}"#, name, genesis_hash, name, validator_registry_str);
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

fn generate_albatross(name: &str, out_dir: &PathBuf, src_dir: &PathBuf, validator_registry_address: Address) {
    info!("Generating Albatross genesis config: {}", name);

    let directory = out_dir.join(name);
    fs::create_dir_all(&directory).unwrap();

    info!("genesis source file: {}", src_dir.join(format!("{}.toml", name)).display());

    let genesis_hash = GenesisBuilder::default()
        .with_validator_registry_address(validator_registry_address.clone())
        .with_config_file(src_dir.join(format!("{}.toml", name))).unwrap()
        .write_to_files(&directory).unwrap();
    write_genesis_rs(&directory, name, &genesis_hash, Some(validator_registry_address));
}


fn main() {
    //setup_panic!();
    simple_logger::init_with_level(Level::Debug)
        .unwrap_or_else(|e| eprintln!("Failed to initialize logging: {}", e));

    let out_dir = Path::new(&env::var("OUT_DIR").unwrap()).join("genesis");
    let src_dir = Path::new("src").join("genesis");

    info!("Taking genesis config files from: {}", src_dir.display());
    info!("Writing genesis data to: {}", out_dir.display());

    generate_powchain("main-powchain", &out_dir, PowChainGenesis::main());
    generate_powchain("test-powchain", &out_dir, PowChainGenesis::test());
    generate_powchain("dev-powchain", &out_dir, PowChainGenesis::dev());
    generate_albatross("dev-albatross", &out_dir, &src_dir,
                       Address::from_str("735b84b129d1ae59bdd1d6b9849915cd8bd3d60a").expect("Invalid staking contract address"));
}
