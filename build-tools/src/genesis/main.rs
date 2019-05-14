use nimiq_build_tools::genesis::albatross::GenesisBuilder;
use log::Level;


fn main() {
    simple_logger::init_with_level(Level::Debug).expect("Failed to initialize logging");

    // TODO: command line interface

    let genesis_hash = GenesisBuilder::default()
        .with_config_file("albatross-testnet.toml").unwrap()
        .write_to_files(".").unwrap();

    println!("Genesis Hash: {}", genesis_hash);
}
