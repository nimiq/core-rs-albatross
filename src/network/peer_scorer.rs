use crate::network::address::peer_address_book::PeerAddressBook;
use crate::network::address::peer_address::PeerAddress;
use crate::network::connection::close_type::CloseType;
use std::sync::Arc;
use parking_lot::RwLock;
use crate::network::ProtocolFlags;
use crate::utils::services::ServiceFlags;

pub struct PeerScorer {
    addresses: Arc<PeerAddressBook>
}

impl PeerScorer {
    pub fn new(addresses: Arc<PeerAddressBook>) -> Self {
        return PeerScorer {
            addresses
        }
    }

    pub fn pick_address(&self) -> Option<Arc<PeerAddress>> {
        let lock = self.addresses.state();
        let mut address_iter = lock.address_iter();
        address_iter.next().cloned()
    }

    pub fn is_good_peer_set(&self) -> bool {
        false
    }

    pub fn needs_good_peers(&self) -> bool {
        false
    }

    pub fn needs_more_peers(&self) -> bool {
        true
    }

    pub fn is_good_peer(&self, peer_address: Arc<PeerAddress>) -> bool {
        true
    }

    pub fn score_connections(&self) {

    }

    pub fn recycle_connections(&self, count: u32, ty: CloseType, reason: &str) {

    }

    pub fn lowest_connection_score(&self) -> Option<f32> {
        None
    }
}
