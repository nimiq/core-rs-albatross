pub use self::time::NetworkTime;

pub mod address;
pub mod message;
pub mod websocket;
pub mod peer_channel;
pub mod peer_scorer;
pub mod time;
pub mod connection;
pub mod peer;

use beserial::{Serialize, Deserialize};

pub use crate::network::peer::Peer;

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum Protocol {
    Dumb = 0,
    Wss = 1,
    Rtc = 2,
    Ws = 4
}

const IPV4_SUBNET_MASK: u8 = 24;
const IPV6_SUBNET_MASK: u8 = 96;
const PEER_COUNT_PER_IP_MAX: usize = 20;
const OUTBOUND_PEER_COUNT_PER_SUBNET_MAX: usize = 2;
const INBOUND_PEER_COUNT_PER_SUBNET_MAX: usize = 100;
const PEER_COUNT_MAX: usize = 4000;
const PEER_COUNT_DUMB_MAX: usize = 1000;
