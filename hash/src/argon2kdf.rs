use argon2::Config;
pub use argon2::Error as Argon2Error;

// Taken from https://github.com/nimiq/core-js/blob/c98d56b2dd967d9a9c9a97fe4c54bfaac743aa0c/src/main/generic/utils/crypto/CryptoWorkerImpl.js#L146
const MEMORY_COST: u32 = 512;

pub fn compute_argon2_kdf(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    derived_key_length: usize,
) -> Result<Vec<u8>, Argon2Error> {
    let config = Config {
        time_cost: iterations,
        hash_length: derived_key_length as u32,
        mem_cost: MEMORY_COST,
        variant: argon2::Variant::Argon2d,
        ..Default::default()
    };

    let mut hash = argon2::hash_raw(password, salt, &config)?;
    assert!(hash.len() >= derived_key_length);
    hash.resize(derived_key_length, 0);
    Ok(hash)
}
