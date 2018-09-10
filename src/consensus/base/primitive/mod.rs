use self::crypto::PublicKey;
use self::hash::{Blake2bHash, Blake2bHasher, Hasher, SerializeContent};
use std::convert::From;
use std::io;

#[macro_use]
pub mod macros;

pub mod crypto;
pub mod hash;
pub mod coin;

create_typed_array!(Address, u8, 20);
hash_typed_array!(Address);

impl From<Blake2bHash> for Address {
    fn from(hash: Blake2bHash) -> Self {
        let hash_arr: [u8; 32] = hash.into();
        return Address::from(&hash_arr[0..Address::len()]);
    }
}

impl<'a> From<&'a PublicKey> for Address {
    fn from(public_key: &'a PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        return Address::from(hash);
    }
}

