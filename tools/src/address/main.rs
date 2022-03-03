extern crate nimiq_keys as keys;

use keys::{Address, PrivateKey, PublicKey, SecureGenerate};

fn main() {
    let private_key = PrivateKey::generate_default_csprng();
    let public_key = PublicKey::from(&private_key);
    let address = Address::from(&public_key);

    println!("Address:       {}", address.to_user_friendly_address());
    println!("Address (raw): {}", address.to_hex());
    println!("Public Key:    {}", public_key.to_hex());
    println!("Private Key:   {}", hex::encode(private_key.as_bytes()));
}
