use beserial::Deserialize;
use clap::{Arg, Command};
use nimiq_keys::{Address, PrivateKey, PublicKey, SecureGenerate};
use std::{error::Error, process};

fn parse_private_key(s: &str) -> Result<PrivateKey, Box<dyn Error>> {
    Ok(PrivateKey::deserialize_from_vec(&hex::decode(s)?)?)
}

fn main() {
    let matches = Command::new("nimiq-address")
        .about("Displays address etc. of a random or specified key")
        .arg(Arg::new("private").takes_value(true).value_name("PRIVATE"))
        .get_matches();

    let private_key = if let Some(p) = matches.value_of("private") {
        match parse_private_key(p) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Error parsing private key: {}", e);
                process::exit(1);
            }
        }
    } else {
        PrivateKey::generate_default_csprng()
    };
    let public_key = PublicKey::from(&private_key);
    let address = Address::from(&public_key);

    println!("Address:       {}", address.to_user_friendly_address());
    println!("Address (raw): {}", address.to_hex());
    println!("Public Key:    {}", public_key.to_hex());
    println!("Private Key:   {}", hex::encode(private_key.as_bytes()));
}
