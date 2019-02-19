pub mod net_address;
pub mod peer_address;
pub mod peer_uri;
pub mod seed_list;

pub use self::net_address::*;
pub use self::peer_address::*;
pub use self::peer_uri::PeerUri;
pub use self::seed_list::SeedList;

use hex::FromHex;

use nimiq_keys::{PublicKey};
use nimiq_hash::{Blake2bHash, Blake2bHasher, Hasher};

create_typed_array!(PeerId, u8, 16);
add_hex_io_fns_typed_arr!(PeerId, PeerId::SIZE);

impl From<Blake2bHash> for PeerId {
    fn from(hash: Blake2bHash) -> Self {
        let hash_arr: [u8; 32] = hash.into();
        PeerId::from(&hash_arr[0..PeerId::len()])
    }
}

impl<'a> From<&'a PublicKey> for PeerId {
    fn from(public_key: &'a PublicKey) -> Self {
        let hash = Blake2bHasher::default().digest(public_key.as_bytes());
        PeerId::from(hash)
    }
}
