#[macro_use]
extern crate derive_builder;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate enum_display_derive;
#[macro_use]
extern crate log;
extern crate rand;

extern crate nimiq_network as network;
extern crate nimiq_consensus as consensus;
extern crate nimiq_database as database;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_primitives as primitives;
extern crate nimiq_mempool as mempool;
extern crate nimiq_utils as utils;
#[cfg(feature="validator")]
extern crate nimiq_validator as validator;
extern crate nimiq_bls as bls;


mod config;
mod error;
mod client;
mod prelude;


// Testing only: Remove later and rename to lib.rs

use config::ClientConfig;
use error::Error;


fn main() {
    let config = ClientConfig::builder()
        .ws("mercury.devnet", None)
        .build()
        .unwrap();

    println!("Config: {:#?}", config);
}
