use std::env;
use std::process::exit;

use nimiq_build_tools::genesis::{GenesisBuilder, GenesisInfo};

fn usage(args: Vec<String>) -> ! {
    eprintln!(
        "Usage: {} GENESIS_FILE",
        args.get(0).unwrap_or(&String::from("nimiq-genesis"))
    );
    exit(1);
}

fn main() {
    pretty_env_logger::init();

    let args = env::args().collect::<Vec<String>>();

    if let Some(file) = args.get(1) {
        let GenesisInfo {
            block,
            hash,
            accounts,
        } = GenesisBuilder::new()
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
