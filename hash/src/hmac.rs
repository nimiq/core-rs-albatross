use crate::{
    sha512::{Sha512Hash, Sha512Hasher, SHA512_LENGTH},
    Hasher,
};

enum Key<'a> {
    Borrowed(&'a [u8]),
    Owned([u8; SHA512_LENGTH]),
}

impl<'a> Key<'a> {
    fn get(&self, index: usize) -> Option<u8> {
        match self {
            Key::Borrowed(key) => key.get(index),
            Key::Owned(key) => key.get(index),
        }
        .cloned()
    }
}

pub fn compute_hmac_sha512(key: &[u8], data: &[u8]) -> Sha512Hash {
    let hashed_key = if key.len() > Sha512Hash::block_size() {
        Key::Owned(Sha512Hasher::default().digest(key).into())
    } else {
        Key::Borrowed(key)
    };

    let mut inner_key: Vec<u8> = Vec::with_capacity(Sha512Hash::block_size());
    let mut outer_key: Vec<u8> = Vec::with_capacity(Sha512Hash::block_size());
    for i in 0..Sha512Hash::block_size() {
        let byte: u8 = hashed_key.get(i).unwrap_or(0);
        inner_key.push(0x36 ^ byte);
        outer_key.push(0x5c ^ byte);
    }

    let inner_hash = Sha512Hasher::default()
        .chain(&inner_key)
        .chain(&data)
        .finish();
    Sha512Hasher::default()
        .chain(&outer_key)
        .chain(&inner_hash)
        .finish()
}
