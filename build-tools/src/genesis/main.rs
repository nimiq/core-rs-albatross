use nimiq_build_tools::genesis::albatross::GenesisBuilder;


fn main() {
    simple_logger::init().expect("Failed to initialize logging");

    // TODO: command line interface

    GenesisBuilder::default()
        .with_config_file("albatross-testnet.toml").unwrap()
        .write_to_files(".").unwrap();
}
