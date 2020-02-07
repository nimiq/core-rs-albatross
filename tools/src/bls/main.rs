extern crate nimiq_bls as bls;

use beserial::Serialize;
use bls::SecureGenerate;
use bls::{PublicKey, SecretKey};

fn main() {
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
}
