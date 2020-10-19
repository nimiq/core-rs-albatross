#![feature(ip)] // For IpAddr::is_global

#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
extern crate nimiq_network_interface as network_interface;

mod behaviour;
mod handler;
mod network;
mod peer;
mod protocol;
//mod peer_address_book;
pub mod discovery;
mod message;
