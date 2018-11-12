pub mod net_address;
pub mod peer_address;

pub use self::net_address::*;
pub use self::peer_address::*;

use consensus::base::primitive::crypto::{PublicKey};
use consensus::base::primitive::hash::{Blake2bHash, Blake2bHasher, Hasher};

create_typed_array!(PeerId, u8, 16);

impl From<Blake2bHash> for PeerId {
    fn from(hash: Blake2bHash) -> Self {
        let hash_arr: [u8; 32] = hash.into();
        return PeerId::from(&hash_arr[0..PeerId::len()]);
    }
}

impl<'a> From<&'a PublicKey> for PeerId {
    fn from(public_key: &'a PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        return PeerId::from(hash);
    }
}
