use libargon2_sys::argon2d_hash;
pub use libargon2_sys::Argon2Error;

// Taken from https://github.com/nimiq/core-js/blob/c98d56b2dd967d9a9c9a97fe4c54bfaac743aa0c/src/main/generic/utils/crypto/CryptoWorkerImpl.js#L146
const MEMORY_COST: u32 = 512;
const PARALELLISM: u32 = 1;

pub fn compute_argon2_kdf(password: &[u8], salt: &[u8], iterations: u32, derived_key_length: usize) -> Result<Vec<u8>, Argon2Error> {
    let mut out = vec![0; derived_key_length];
    argon2d_hash(iterations, MEMORY_COST, PARALELLISM, password, salt, out.as_mut(), 0)
        .map(|_| out)
}
