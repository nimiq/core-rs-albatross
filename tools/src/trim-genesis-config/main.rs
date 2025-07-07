use std::process;

use clap::{Arg, ArgAction, Command};
use nimiq_database::mdbx::{DatabaseConfig, MdbxDatabase};
use nimiq_genesis_builder::{config::GenesisConfig, GenesisBuilder};

fn db() -> MdbxDatabase {
    MdbxDatabase::new_volatile(DatabaseConfig {
        size: Some(0..100 * 1024 * 1024 * 1024),
        ..Default::default()
    })
    .expect("couldn't open volatile database")
}

fn main() {
    let matches = Command::new("nimiq-trim-genesis-config")
        .version(nimiq_utils::CARGO_VERSION)
        .about("Trims genesis config to not contain the state")
        .arg(
            Arg::new("genesis-config")
                .value_name("GENESIS_INFO")
                .help("Path to genesis config toml")
                .required(true),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .action(ArgAction::SetTrue)
                .help("Show status information"),
        )
        .arg(
            Arg::new("no-verify")
                .long("no-verify")
                .action(ArgAction::SetTrue)
                .help("Don't check that the generated config file is equivalent"),
        )
        .get_matches();

    let genesis_config = matches.get_one::<String>("genesis-config").unwrap();
    let verbose = matches.get_flag("verbose");
    let no_verify = matches.get_flag("no-verify");

    macro_rules! log {
        ($($args:tt)*) => {
            if verbose {
                eprintln!($($args)*);
            }
        }
    }

    log!("reading genesis config...");
    let genesis_builder = GenesisBuilder::from_config_file(genesis_config).unwrap();
    log!("generating genesis block...");
    let genesis_info = genesis_builder.generate(db()).unwrap();
    let hash = genesis_info.block.hash();
    log!("{}: generated genesis block hash", hash);
    log!("generating config...");
    let genesis_config_trimmed =
        GenesisConfig::trimmed_from_genesis(&genesis_info.block.unwrap_macro().header);

    if !no_verify {
        log!("reading trimmed config...");
        let genesis_builder = GenesisBuilder::from_config(genesis_config_trimmed.clone()).unwrap();
        log!("generating genesis block from trimmed config...");
        let genesis_info = genesis_builder.generate(db()).unwrap();
        let hash_trimmed = genesis_info.block.hash();
        log!("{}: genesis block hash generated from trimmed config", hash);
        if hash_trimmed == hash {
            log!("hashes match");
        } else {
            eprintln!("hash mismatch, {hash_trimmed} != {hash}");
            eprintln!("aborting");
            process::exit(1);
        }
    }
    log!("done");
    log!("");

    print!("{}", toml::to_string(&genesis_config_trimmed).unwrap());
}
