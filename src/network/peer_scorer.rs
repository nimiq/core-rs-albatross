use crate::network::address::peer_address_book::PeerAddressBook;
use crate::network::address::peer_address::PeerAddress;
use std::sync::Arc;

pub struct PeerScorer<'t> {
    addresses: &'t PeerAddressBook
}

impl<'t> PeerScorer<'t> {
    pub fn new(addresses: &'t PeerAddressBook) -> PeerScorer {
        return PeerScorer {
            addresses
        }
    }

    pub fn pick_address(&self) -> Option<Arc<PeerAddress>> {
        // TODO
        let ws_addrs = self.addresses.get_ws();
        for ws_addr in ws_addrs {
            return Some(Arc::clone(ws_addr));
        }
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
