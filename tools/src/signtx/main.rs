use std::io::stdin;
use std::process::exit;
use std::str::FromStr;

use anyhow::Error;
use clap::{crate_authors, crate_description, crate_version, Arg, Command};
use thiserror::Error;

use beserial::{Deserialize, Serialize};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_transaction::Transaction;

fn run_app() -> Result<(), Error> {
    let matches = Command::new("Sign transaction")
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(
            Arg::new("secret_key")
                .short('k')
                .long("secret-key")
                .value_name("SECRET_KEY")
                .help("Specify the secret key to be used to sign the transaction.")
                .takes_value(true),
        )
        .arg(
            Arg::new("tx_from_stdin")
                .long("stdin")
                .help("Read transaction as hex from STDIN")
                .takes_value(false),
        )
        .arg(
            Arg::new("from_address")
                .short('f')
                .long("from")
                .value_name("ADDRESS")
                .help("Send transaction from ADDRESS.")
                .takes_value(true),
        )
        .arg(
            Arg::new("to_address")
                .short('t')
                .long("to")
                .value_name("ADDRESS")
                .help("Send transaction to ADDRESS.")
                .takes_value(true),
        )
        .arg(
            Arg::new("value")
                .short('v')
                .long("value")
                .value_name("VALUE")
                .help("Send transaction with VALUE amount.")
                .takes_value(true),
        )
        .arg(
            Arg::new("fee")
                .short('F')
                .long("fee")
                .value_name("VALUE")
                .help("Send transaction with VALUE fee.")
                .takes_value(true),
        )
        .arg(
            Arg::new("validity_start_height")
                .short('V')
                .long("validity-start-height")
                .value_name("HEIGHT")
                .help("Set validity start height")
                .takes_value(true),
        )
        .arg(
            Arg::new("network_id")
                .short('N')
                .long("network")
                .value_name("NETWORK")
                .help("Set network ID")
                .takes_value(true),
        )
        .get_matches();

    // read transaction either from arguments or stdin
    let tx = if matches.is_present("tx_from_stdin") {
        let mut line = String::new();
        stdin().read_line(&mut line)?;
        Transaction::deserialize_from_vec(&hex::decode(&line)?)?
    } else {
        let from_address = Address::from_user_friendly_address(
            matches
                .value_of("from_address")
                .ok_or(AppError::SenderAddress)?,
        )?;
        let to_address = Address::from_user_friendly_address(
            matches
                .value_of("to_address")
                .ok_or(AppError::RecipientAddress)?,
        )?;
        let value = Coin::from_str(matches.value_of("value").ok_or(AppError::Value)?)?;
        let fee = Coin::from_str(matches.value_of("fee").ok_or(AppError::Fee)?)?;
        let validity_start_height = u32::from_str(
            matches
                .value_of("validity_start_height")
                .ok_or(AppError::ValidityStartHeight)?,
        )?;
        let network_id = match matches.value_of("network") {
            Some(s) => NetworkId::from_str(s)?,
            None => NetworkId::Main,
        };
        Transaction::new_basic(
            from_address,
            to_address,
            value,
            fee,
            validity_start_height,
            Some(network_id),
        )
    };

    // sign transaction
    if let Some(hex_secret_key) = matches.value_of("secret_key") {
        let raw_secret_key = hex::decode(hex_secret_key)?;
        let key_pair: KeyPair = PrivateKey::deserialize_from_vec(&raw_secret_key)?.into();
        let signature = key_pair.sign(tx.serialize_content().as_slice());
        let raw_signature = signature.serialize_to_vec();
        println!("{}", hex::encode(&raw_signature));
        Ok(())
    } else {
        Err(AppError::SecretKey.into())
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

#[derive(Debug, Error)]
enum AppError {
    #[error("Secret key is missing")]
    SecretKey,
    #[error("Sender address is missing")]
    SenderAddress,
    #[error("Recipient address is missing")]
    RecipientAddress,
    #[error("Transaction value is missing")]
    Value,
    #[error("Transaction fee is missing")]
    Fee,
    #[error("Validity start height is missing")]
    ValidityStartHeight,
}
