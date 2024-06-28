use std::{env, process::exit};

use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis_builder::{GenesisBuilder, GenesisInfo};

fn usage(args: Vec<String>) -> ! {
    eprintln!(
        "Usage: {} GENESIS_FILE",
        args.first().unwrap_or(&String::from("nimiq-genesis"))
    );
    exit(1);
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let db =
        MdbxDatabase::new_volatile(Default::default()).expect("Could not open a volatile database");
    let args = env::args().collect::<Vec<String>>();

    if let Some(file) = args.get(1) {
        let GenesisInfo {
            block,
            hash,
            accounts,
        } = GenesisBuilder::from_config_file(file)
            .unwrap()
            .generate(db)
            .unwrap();

        println!("Genesis Block: {hash}");
        println!("{block:#?}");
        println!();
        println!("Genesis Accounts:");
        println!("{accounts:#?}");
    } else {
        usage(args);
    }
}
