#[macro_use]
pub mod macros;

pub mod crypto;
pub mod hash;

use std::convert::From;
use self::hash::Blake2bHash;
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
        unimplemented!();
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
//        let mut blake2b_hasher = hash::Blake2bHasher;
//        blake2b_hasher.write(public_key.into()[0..32]);
//        let hash: [u8; 32] = blake2b_hasher.finish().into();
        let hash: [u8; 32] = [0; 32];
        return PeerId::from(&hash[0..PeerId::len()]);
    }
}

