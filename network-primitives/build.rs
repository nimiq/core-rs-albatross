#[macro_use]
extern crate human_panic;
#[macro_use]
extern crate log;

use std::env;
use std::path::{Path, PathBuf};
use std::fs;

use nimiq_build_tools::genesis::albatross::GenesisBuilder;
use nimiq_build_tools::genesis::powchain::PowChainGenesis;
use log::Level;
use nimiq_hash::Blake2bHash;


fn write_genesis_rs(directory: &PathBuf, name: &str, genesis_hash: &Blake2bHash) {
    let genesis_rs = format!(r#"GenesisData {{
            block: include_bytes!(concat!(env!("OUT_DIR"), "/genesis/{}/block.dat")),
            hash: "{}".into(),
            accounts: include_bytes!(concat!(env!("OUT_DIR"), "/genesis/{}/accounts.dat")),
    }}"#, name, genesis_hash, name);
    debug!("Writing genesis source code: {}", &genesis_rs);
    fs::write(directory.join("genesis.rs"), genesis_rs.as_bytes()).unwrap();
}

fn generate_powchain(name: &str, out_dir: &PathBuf, powchain: PowChainGenesis) {
    let directory = out_dir.join(name);
    fs::create_dir_all(&directory).unwrap();
    let genesis_hash = powchain.generate_genesis_hash().unwrap();

    powchain.write_to_files(&directory).unwrap();
    write_genesis_rs(&directory, name, &genesis_hash);
}

fn generate_albatross(name: &str, out_dir: &PathBuf, src_dir: &PathBuf) {
    info!("Generating Albatross genesis config: {}", name);

    let directory = out_dir.join(name);
    fs::create_dir_all(&directory).unwrap();

    let genesis_hash = GenesisBuilder::default()
        .with_config_file(src_dir.join(format!("{}.toml", name))).unwrap()
        .write_to_files(&directory).unwrap();
    write_genesis_rs(&directory, name, &genesis_hash);
}


fn main() {
    setup_panic!();
    simple_logger::init_with_level(Level::Debug)
        .unwrap_or_else(|e| eprintln!("Failed to initialize logging: {}", e));

    let out_dir = Path::new(&env::var("OUT_DIR").unwrap()).join("genesis");
    let src_dir = Path::new("src").join("genesis");

    info!("Taking genesis config files from: {}", src_dir.display());
    info!("Writing genesis data to: {}", out_dir.display());

    generate_powchain("main-powchain", &out_dir, PowChainGenesis::main());
    generate_powchain("test-powchain", &out_dir, PowChainGenesis::test());
    generate_powchain("dev-powchain", &out_dir, PowChainGenesis::dev());
    generate_albatross("test-albatross", &out_dir, &src_dir);
}
