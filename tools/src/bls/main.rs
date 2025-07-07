use clap::Parser;
use nimiq_bls::{PublicKey, SecretKey};
use nimiq_serde::Serialize;
use nimiq_utils::key_rng::SecureGenerate;

/// Generates a random BLS keypair and its proof of knowledge.
#[derive(Debug, Parser)]
#[command(name = "nimiq-bls", version = nimiq_utils::CARGO_VERSION)]
struct Args {}

fn main() {
    Args::parse();

    let secret_key = SecretKey::generate_default_csprng();
    let public_key = PublicKey::from_secret(&secret_key);

    println!("# Public Key:");
    println!();
    let x: Vec<u8> = public_key.serialize_to_vec(); // need to apply a little bit of force to make it a slice
    println!("{}", hex::encode(x));
    println!();
    println!("# Secret Key:");
    println!();
    println!("{}", hex::encode(secret_key.serialize_to_vec()));
    println!();
    println!("# Proof Of Knowledge:");
    println!();
    println!("{}", hex::encode(secret_key.sign(&public_key).compress()));
}
