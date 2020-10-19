#![feature(ip)] // For IpAddr::is_global

#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate bitflags;
extern crate nimiq_bls as bls;
extern crate nimiq_hash as hash;
extern crate nimiq_hash_derive as hash_derive;
extern crate nimiq_keys as keys;
#[macro_use]
extern crate nimiq_macros as macros;
extern crate nimiq_primitives as primitives;
extern crate nimiq_utils as utils;

pub mod address;
pub mod protocol;
pub mod services;
pub mod version;
//pub mod peer_info;
