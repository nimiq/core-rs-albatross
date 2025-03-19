use nimiq_bls::*;
use nimiq_utils::key_rng::SecureGenerate;

fn main() {
    let keypair = KeyPair::generate(&mut rand::rng());
    let message = "Who is on first.".to_string();
    let sig = keypair.sign(&message);
    println!("Signature:\n {sig}");
}
