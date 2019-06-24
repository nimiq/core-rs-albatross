#[macro_use]
extern crate clap;
extern crate failure;
extern crate hex;
extern crate beserial;
extern crate nimiq_transaction as transaction;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;

use std::io::stdin;
use std::process::exit;
use std::str::FromStr;
use failure::Fail;
use clap::{App, Arg};
use keys::{PrivateKey, KeyPair, Address};
use transaction::Transaction;
use beserial::{Serialize, Deserialize};
use failure::Error;
use primitives::coin::Coin;


fn run_app() -> Result<(), Error> {
    let matches = App::new("Sign transaction")
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(Arg::with_name("secret_key")
            .short("k")
            .long("secret-key")
            .value_name("SECRET_KEY")
            .help("Specify the secret key to be used to sign the transaction.")
            .takes_value(true))
        .arg(Arg::with_name("tx_from_stdin")
            .long("stdin")
            .help("Read transaction as hex from STDIN")
            .takes_value(false))
        .arg(Arg::with_name("from_address")
            .short("f")
            .long("from")
            .value_name("ADDRESS")
            .help("Send transaction from ADDRESS.")
            .takes_value(true))
        .arg(Arg::with_name("to_address")
            .short("t")
            .long("to")
            .value_name("ADDRESS")
            .help("Send transaction to ADDRESS.")
            .takes_value(true))
        .arg(Arg::with_name("value")
            .short("v")
            .long("value")
            .value_name("VALUE")
            .help("Send transaction with VALUE amount.")
            .takes_value(true))
        .arg(Arg::with_name("fee")
            .short("F")
            .long("fee")
            .value_name("VALUE")
            .help("Send transaction with VALUE fee.")
            .takes_value(true))
        .arg(Arg::with_name("validity_start_height")
            .short("V")
            .long("validity-start-height")
            .value_name("HEIGHT")
            .help("Set validity start height")
            .takes_value(true))
        .arg(Arg::with_name("network_id")
            .short("N")
            .long("network")
            .value_name("NETWORK")
            .help("Set network ID")
            .takes_value(true))
        .get_matches();

    // read transaction either from arguments or stdin
    let tx = if matches.is_present("tx_from_stdin") {
        let mut line= String::new();
        stdin().read_line(&mut line)?;
        Transaction::deserialize_from_vec(&hex::decode(&line)?)?
    }
    else {
        let from_address = Address::from_user_friendly_address(matches.value_of("from_address")
            .ok_or(AppError::MissingSenderAddress)?)?;
        let to_address = Address::from_user_friendly_address(matches.value_of("to_address")
            .ok_or(AppError::MissingRecipientAddress)?)?;
        let value = Coin::from_str(matches.value_of("value")
            .ok_or(AppError::MissingValue)?)?;
        let fee = Coin::from_str(matches.value_of("fee")
            .ok_or(AppError::MissingFee)?)?;
        let validity_start_height= u32::from_str(matches.value_of("validity_start_height")
            .ok_or(AppError::MissingValidityStartHeight)?)?;
        let network_id = match matches.value_of("network") {
            Some(s) => NetworkId::from_str(s)?,
            None => NetworkId::Main
        };
        Transaction::new_basic(from_address, to_address, value, fee, validity_start_height, network_id)
    };

    // sign transaction
    if let Some(hex_secret_key) = matches.value_of("secret_key") {
        let raw_secret_key = hex::decode(hex_secret_key)?;
        let key_pair: KeyPair = PrivateKey::deserialize_from_vec(&raw_secret_key)?.into();
        let signature = key_pair.sign(tx.serialize_content().as_slice());
        let raw_signature = signature.serialize_to_vec();
        println!("{}", hex::encode(&raw_signature));
        Ok(())
    }
    else {
        Err(AppError::MissingSecretKey)?
    }
}

fn main() {
    exit(match run_app() {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("Error: {}", e);
            1
        }
    });
}


use primitives::networks::NetworkId;


#[derive(Debug, Fail)]
enum AppError {
    #[fail(display = "Secret key is missing")]
    MissingSecretKey,
    #[fail(display = "Sender address is missing")]
    MissingSenderAddress,
    #[fail(display = "Recipient address is missing")]
    MissingRecipientAddress,
    #[fail(display = "Transaction value is missing")]
    MissingValue,
    #[fail(display = "Transaction fee is missing")]
    MissingFee,
    #[fail(display = "Validity start height is missing")]
    MissingValidityStartHeight
}
