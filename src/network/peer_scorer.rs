use crate::network::address::peer_address_book::PeerAddressBook;
use crate::network::address::peer_address::PeerAddress;
use std::sync::Arc;
use parking_lot::RwLock;

pub struct PeerScorer {
    addresses: Arc<RwLock<PeerAddressBook>>
}

impl PeerScorer {
    pub fn new(addresses: Arc<RwLock<PeerAddressBook>>) -> Self {
        return PeerScorer {
            addresses
        }
    }

    pub fn pick_address(&self) -> Option<Arc<PeerAddress>> {
        // TODO
        return None;
    }

    pub fn is_good_peer_set(&self) {
        unimplemented!()
    }

    pub fn needs_more_peers(&self) {
        unimplemented!()
    }

    pub fn is_good_peer(&self, peer_address: Arc<PeerAddress>) {
        unimplemented!()
    }
}
