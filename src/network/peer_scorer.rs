use crate::network::address::peer_address_book::PeerAddressBook;
use crate::network::address::peer_address::PeerAddress;
use crate::network::connection::close_type::CloseType;
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
        unimplemented!()
    }

    pub fn is_good_peer_set(&self) -> bool {
        unimplemented!()
    }

    pub fn needs_good_peers(&self) -> bool {
        unimplemented!()
    }

    pub fn needs_more_peers(&self) -> bool {
        unimplemented!()
    }

    pub fn is_good_peer(&self, peer_address: Arc<PeerAddress>) -> bool {
        unimplemented!()
    }

    pub fn score_connections(&self) {
        unimplemented!()
    }

    pub fn recycle_connections(&self, count: u32, ty: CloseType, reason: &str) {
        unimplemented!()
    }

    pub fn lowest_connection_score(&self) -> Option<f32> {
        unimplemented!()
    }
}
