use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures_03::Stream;

use hash::Blake2bHash;
use network_interface::prelude::{CloseReason, Message, Peer as PeerInterface, SendError};
use network_interface::peer::RequestResponse;
use peer_address::address::NetAddress;
use peer_address::address::PeerAddress;

use crate::connection::close_type::CloseType;
use crate::peer_channel::PeerChannel;
use crate::network::NetworkError;

#[derive(Clone, Debug)]
pub struct Peer {
    pub channel: Arc<PeerChannel>,
    pub version: u32,
    pub head_hash: Blake2bHash,
    pub time_offset: i64,
    pub user_agent: Option<String>,
}

impl Peer {
    pub fn new(channel: Arc<PeerChannel>, version: u32, head_hash: Blake2bHash, time_offset: i64, user_agent: Option<String>) -> Self {
        Peer {
            channel,
            version,
            head_hash,
            time_offset,
            user_agent,
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

impl fmt::Display for Peer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.peer_address())
    }
}

impl PartialEq for Peer {
    fn eq(&self, other: &Peer) -> bool {
        self.channel == other.channel
    }
}

impl Eq for Peer {}

impl Hash for Peer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.channel.hash(state);
    }
}

#[async_trait]
impl PeerInterface for Peer {
    type Id = Arc<PeerAddress>;
    type Error = NetworkError;

    fn id(&self) -> Self::Id {
        self.channel.address_info.peer_address().expect("PeerAddress not set")
    }

    async fn send<T: Message>(&self, msg: &T) -> Result<(), SendError> {
        self.channel.peer_sink.send(msg).map_err(|_| SendError::AlreadyClosed)
    }

    fn receive<T: Message + 'static>(&self) -> Pin<Box<dyn Stream<Item = T> + Send>> {
        self.channel.receive()
    }

    fn close(&self, _ty: CloseReason) {
        self.channel.close(CloseType::Unknown);
    }

    async fn request<R: RequestResponse>(&self, _request: &<R as RequestResponse>::Request) -> Result<R::Response, Self::Error> {
        unimplemented!()
    }

    fn requests<R: RequestResponse>(&self) -> Box<dyn Stream<Item = R::Request>> {
        unimplemented!()
    }
}
