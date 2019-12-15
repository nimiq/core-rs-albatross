use argon2;

pub use argon2::Error as Argon2Error;
use argon2::{Config, ThreadMode};

// Taken from https://github.com/nimiq/core-js/blob/c98d56b2dd967d9a9c9a97fe4c54bfaac743aa0c/src/main/generic/utils/crypto/CryptoWorkerImpl.js#L146
const MEMORY_COST: u32 = 512;
const PARALELLISM: u32 = 1;


pub fn compute_argon2_kdf(password: &[u8], salt: &[u8], iterations: u32, derived_key_length: usize) -> Result<Vec<u8>, Argon2Error> {
    let mut config = Config::default();
    config.time_cost = iterations;
    config.hash_length = derived_key_length as u32;
    config.mem_cost = MEMORY_COST;
    config.thread_mode = ThreadMode::from_threads(PARALELLISM);
    config.variant = argon2::Variant::Argon2d;

    let mut hash = argon2::hash_raw(password, salt, &config)?;
    assert!(hash.len() >= derived_key_length);
    hash.resize(derived_key_length, 0);
    Ok(hash)
}
