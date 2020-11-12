#![feature(ip)] // For IpAddr::is_global

#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;

mod behaviour;
mod limit;
pub mod message;
mod network;
pub mod discovery;
pub mod tagged_signing;
pub mod message_codec;


pub const MESSAGE_PROTOCOL: &[u8] = b"/nimiq/message/0.0.1";
pub const DISCOVERY_PROTOCOL: &[u8] = b"/nimiq/discovery/0.0.1";
pub const LIMIT_PROTOCOL: &[u8] = b"/nimiq/limit/0.0.1";
