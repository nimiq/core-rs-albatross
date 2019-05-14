extern crate nimiq_bls as bls;

use rand::rngs::OsRng;

use beserial::Serialize;
use bls::bls12_381::{PublicKey, SecretKey};

fn main() {
    let mut csprng = OsRng::new().expect("OS RNG not available");
    let secret_key = SecretKey::generate(&mut csprng);
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