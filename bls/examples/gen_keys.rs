use nimiq_bls::*;
use nimiq_utils::key_rng::SecureGenerate;

fn main() {
    let keypair = KeyPair::generate(&mut rand::rng());
    println!("Secret key:\n {}", keypair.secret_key);
    println!("Public key:\n {}", keypair.public_key);
}
