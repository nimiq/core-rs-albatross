use std::{io::stdin, process::exit, str::FromStr};

use anyhow::Error;
use clap::{
    crate_authors, crate_description, crate_version, value_parser, Arg, ArgAction, Command,
};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_primitives::{coin::Coin, networks::NetworkId};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::Transaction;
use thiserror::Error;

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
                .help("Specify the secret key to be used to sign the transaction."),
        )
        .arg(
            Arg::new("tx_from_stdin")
                .long("stdin")
                .help("Read transaction as hex from STDIN")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("from_address")
                .short('f')
                .long("from")
                .value_name("ADDRESS")
                .help("Send transaction from ADDRESS."),
        )
        .arg(
            Arg::new("to_address")
                .short('t')
                .long("to")
                .value_name("ADDRESS")
                .help("Send transaction to ADDRESS."),
        )
        .arg(
            Arg::new("value")
                .short('v')
                .long("value")
                .value_name("VALUE")
                .help("Send transaction with VALUE amount."),
        )
        .arg(
            Arg::new("fee")
                .short('F')
                .long("fee")
                .value_name("VALUE")
                .help("Send transaction with VALUE fee."),
        )
        .arg(
            Arg::new("validity_start_height")
                .short('H')
                .long("validity-start-height")
                .value_name("HEIGHT")
                .value_parser(value_parser!(u32))
                .help("Set validity start height"),
        )
        .arg(
            Arg::new("network_id")
                .short('N')
                .long("network")
                .value_name("NETWORK")
                .help("Set network ID"),
        )
        .get_matches();

    // read transaction either from arguments or stdin
    let tx = if matches.get_flag("tx_from_stdin") {
        let mut line = String::new();
        stdin().read_line(&mut line)?;
        Transaction::deserialize_from_vec(&hex::decode(line.trim_end())?)?
    } else {
        let from_address = Address::from_user_friendly_address(
            matches
                .get_one::<String>("from_address")
                .ok_or(AppError::SenderAddress)?,
        )?;
        let to_address = Address::from_user_friendly_address(
            matches
                .get_one::<String>("to_address")
                .ok_or(AppError::RecipientAddress)?,
        )?;
        let value = Coin::from_str(matches.get_one::<String>("value").ok_or(AppError::Value)?)?;
        let fee = Coin::from_str(matches.get_one::<String>("fee").ok_or(AppError::Fee)?)?;
        let validity_start_height = matches
            .get_one::<u32>("validity_start_height")
            .ok_or(AppError::ValidityStartHeight)?;
        let network_id = match matches.get_one::<String>("network_id") {
            Some(s) => NetworkId::from_str(s)?,
            None => NetworkId::Main,
        };
        Transaction::new_basic(
            from_address,
            to_address,
            value,
            fee,
            *validity_start_height,
            network_id,
        )
    };

    // sign transaction
    if let Some(hex_secret_key) = matches.get_one::<String>("secret_key") {
        let raw_secret_key = hex::decode(hex_secret_key)?;
        let key_pair: KeyPair = PrivateKey::deserialize_from_vec(&raw_secret_key)?.into();
        let signature = key_pair.sign(&tx.serialize_content());
        let raw_signature = signature.serialize_to_vec();
        println!("{}", hex::encode(raw_signature));
        Ok(())
    } else {
        Err(AppError::SecretKey.into())
    }
}

fn main() {
    exit(match run_app() {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("Error: {e}");
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
