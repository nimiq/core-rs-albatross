#![feature(ip)] // For IpAddr::is_global

#[macro_use]
extern crate beserial_derive;

#[macro_use]
extern crate nimiq_macros;

mod behaviour;
pub mod codecs;
mod config;
mod connection_pool;
pub mod discovery;
pub mod dispatch;
mod error;
pub mod message_codec;
mod network;
pub mod peer;

pub const MESSAGE_PROTOCOL: &[u8] = b"/nimiq/message/0.0.1";
pub const DISCOVERY_PROTOCOL: &[u8] = b"/nimiq/discovery/0.0.1";

pub use libp2p::{self, core::network::NetworkInfo, identity::Keypair, Multiaddr, PeerId};

pub use config::Config;
pub use error::NetworkError;
pub use network::Network;
