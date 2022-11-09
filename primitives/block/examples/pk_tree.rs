use nimiq_block::MacroBlock;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_keys::{Address, KeyPair as SchnorrKeyPair, SecureGenerate};
use nimiq_primitives::policy;
use nimiq_primitives::slots::{Validators, ValidatorsBuilder};
use nimiq_test_utils::zkp_test_data::get_base_seed;

// Benchmarks for the pk tree root computation.
// Calculating the pk tree root for a large number of validators is an expensive task.

/// Generate validators for the benchmarks.
fn generate_uncompressed_validators() -> Validators {
    let mut rng = get_base_seed();
    let mut validators = ValidatorsBuilder::new();

    for _ in 0..policy::SLOTS {
        let keypair = SchnorrKeyPair::generate(&mut rng);
        let address = Address::from(&keypair.public);
        let bls_keypair = BlsKeyPair::generate(&mut rng);
        validators.push(address, bls_keypair.public_key, keypair.public);
    }

    validators.build()
}

fn main() {
    let validators = generate_uncompressed_validators();

    for i in 0..50 {
        println!("{}", i);
        MacroBlock::pk_tree_root(&validators);
    }
}
