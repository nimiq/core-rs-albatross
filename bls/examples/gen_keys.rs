use nimiq_bls::*;
use nimiq_utils::key_rng::SecureGenerate;
use rand::thread_rng;

fn main() {
    let rng = &mut thread_rng();

    let keypair = KeyPair::generate(rng);

    println!("Secret key:\n {}", keypair.secret_key);

    println!("Public key:\n {}", keypair.public_key);
}
