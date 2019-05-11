extern crate nimiq_bls as bls;

use bls::bls12_381::{SecretKey, PublicKey};
use bls::Encoding;
use rand::rngs::OsRng;

fn main() {
    let mut csprng = OsRng::new().expect("OS RNG not available");
    let secret_key = SecretKey::generate(&mut csprng);
    let public_key = PublicKey::from_secret(&secret_key);

    println!("# Public Key:");
    println!();
    let x: &[u8] = &public_key.to_bytes(); // need to apply a little bit of force to make it a slice
    println!("{}", hex::encode(x));
    println!();
    println!("# Secret Key:");
    println!();
    println!("{}", hex::encode(&secret_key.to_bytes()));
}