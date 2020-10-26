#![feature(ip)] // For IpAddr::is_global

#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
extern crate nimiq_network_interface as network_interface;

mod behaviour;
mod handler;
mod limit;
mod message;
mod network;
mod peer;
mod protocol;
pub mod discovery;
pub mod tagged_signing;
pub mod message_codec;