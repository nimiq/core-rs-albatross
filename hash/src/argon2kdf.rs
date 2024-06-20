use argon2::Config;
pub use argon2::{Error as Argon2Error, Variant as Argon2Variant};

// Taken from https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html#argon2id, 2024-06-20.
const MEMORY_COST: u32 = 12288;

pub fn compute_argon2_kdf(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    derived_key_length: usize,
    variant: Argon2Variant,
) -> Result<Vec<u8>, Argon2Error> {
    let config = Config {
        time_cost: iterations,
        hash_length: derived_key_length as u32,
        mem_cost: MEMORY_COST,
        variant,
        ..Default::default()
    };

    let mut hash = argon2::hash_raw(password, salt, &config)?;
    assert!(hash.len() >= derived_key_length);
    hash.resize(derived_key_length, 0);
    Ok(hash)
}
