use crate::consensus::base::primitive::hash::Argon2dHash;
use std::sync::Arc;
use crate::network::address::peer_address::PeerAddress;
use crate::network::address::net_address::NetAddress;
use crate::network::peer_channel::PeerChannel;

#[derive(Clone, Debug)]
pub struct Peer {
    pub channel: PeerChannel,
    pub version: Option<u8>,
    pub head_hash: Option<Argon2dHash>,
    pub time_offset: Option<u8>,
}

impl Peer {
    pub fn new(channel: PeerChannel) -> Self {
        Peer {
            channel,
            version: None,
            head_hash: None,
            time_offset: None,
        }
    }

    pub fn peer_address(&self) -> Arc<PeerAddress> {
        // If a peer object exists, peer_address should be set.
        self.channel.address_info.peer_address().unwrap()
    }

    pub fn net_address(&self) -> Option<Arc<NetAddress>> {
        self.channel.address_info.net_address()
    }
}
