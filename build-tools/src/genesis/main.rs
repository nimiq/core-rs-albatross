use std::env;
use std::process::exit;

use log::Level;

use nimiq_build_tools::genesis::albatross::{GenesisBuilder, GenesisInfo};

fn usage(args: Vec<String>) -> ! {
    eprintln!(
        "Usage: {} GENESIS_FILE",
        args.get(0).unwrap_or(&String::from("nimiq-genesis"))
    );
    exit(1);
}

fn main() {
    simple_logger::init_with_level(Level::Debug).expect("Failed to initialize logging");

    let args = env::args().collect::<Vec<String>>();

    if let Some(file) = args.get(1) {
        let GenesisInfo {
            block,
            hash,
            accounts,
        } = GenesisBuilder::default()
            .with_config_file(file)
            .unwrap()
            .generate()
            .unwrap();

        println!("Genesis Block: {}", hash);
        println!("{:#?}", block);
        println!();
        println!("Genesis Accounts:");
        println!("{:#?}", accounts);
    } else {
        usage(args);
    }
}
