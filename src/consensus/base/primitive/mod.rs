#[macro_use]
pub mod macros;

pub mod crypto;
pub mod hash;

use std::convert::From;
use hex::{FromHex,FromHexError};
use self::hash::{Blake2bHash, Blake2bHasher, Hasher};
use self::crypto::PublicKey;

create_typed_array!(Address, u8, 20);
create_typed_array!(PeerId, u8, 16);

impl From<Blake2bHash> for Address {
    fn from(hash: Blake2bHash) -> Self {
        let hash_arr: [u8; 32] = hash.into();
        return Address::from(&hash_arr[0..Address::len()]);
    }
}

impl From<PublicKey> for Address {
    fn from(public_key: PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        return Address::from(hash);
    }
}

impl From<Blake2bHash> for PeerId {
    fn from(hash: Blake2bHash) -> Self {
        let hash_arr: [u8; 32] = hash.into();
        return PeerId::from(&hash_arr[0..PeerId::len()]);
    }
}

impl From<PublicKey> for PeerId {
    fn from(public_key: PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        return PeerId::from(hash);
    }
}

