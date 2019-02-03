#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
#[macro_use]
extern crate macros;

pub mod address;
pub mod websocket;
pub mod peer_channel;
pub mod peer_scorer;
pub mod connection;
pub mod peer;
pub mod network_config;
pub mod network;

use beserial::{Serialize, Deserialize};

pub use crate::peer::Peer;
pub use crate::network::{Network, NetworkEvent};
pub use crate::network_config::NetworkConfig;
