use crate::consensus::base::primitive::hash::Blake2bHash;
use std::sync::Arc;
use crate::network::address::peer_address::PeerAddress;
use crate::network::address::net_address::NetAddress;
use crate::network::peer_channel::PeerChannel;

#[derive(Clone, Debug)]
pub struct Peer {
    pub channel: PeerChannel,
    pub version: u32,
    pub head_hash: Blake2bHash,
    pub time_offset: u64,
}

impl Peer {
    pub fn new(channel: PeerChannel, version: u32, head_hash: Blake2bHash, time_offset: u64) -> Self {
        Peer {
            channel,
            version,
            head_hash,
            time_offset,
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
