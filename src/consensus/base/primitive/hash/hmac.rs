use super::{Sha512Hasher, Sha512Hash, Hasher, SHA512_LENGTH};

pub fn compute_hmac_sha512(key: &[u8], data: &[u8]) -> Sha512Hash {
    let mut hashed_key: Option<[u8; SHA512_LENGTH]> = None;
    if key.len() > Sha512Hash::block_size() {
        hashed_key = Some(Sha512Hasher::default().digest(key).into());
    }

    let mut inner_key: Vec<u8> = Vec::with_capacity(Sha512Hash::block_size());
    let mut outer_key: Vec<u8> = Vec::with_capacity(Sha512Hash::block_size());
    for i in 0..Sha512Hash::block_size() {
        // TODO: check if this can be made more efficient.
        let byte: u8 = if let Some(k) = hashed_key { *k.get(i).unwrap_or(&0) } else { *key.get(i).unwrap_or(&0) };
        inner_key.push(0x36 ^ byte);
        outer_key.push(0x5c ^ byte);
    }

    let inner_hash = Sha512Hasher::default().chain(&inner_key).chain(&data).finish();
    Sha512Hasher::default().chain(&outer_key).chain(&inner_hash).finish()
}
