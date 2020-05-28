use rand::thread_rng;

use nimiq_bls::*;
use nimiq_utils::key_rng::SecureGenerate;

fn main() {
    let rng = &mut thread_rng();

    let keypair = KeyPair::generate(rng);

    let message = format!("Who is on first.");

    let sig = keypair.sign(&message);

    println!("Signature:\n {}", sig);
}
