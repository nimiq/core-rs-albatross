#![feature(ip)] // For Ip::is_global

#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate nimiq_macros as macros;
extern crate nimiq_account as account;
extern crate nimiq_block as block;
extern crate nimiq_block_albatross as block_albatross;
extern crate nimiq_bls as bls;
extern crate nimiq_hash as hash;
extern crate nimiq_hash_derive as hash_derive;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;

#[cfg(feature = "address")]
pub mod address;
#[cfg(feature = "services")]
pub mod services;
#[cfg(feature = "version")]
pub mod version;

#[cfg(feature = "networks")]
pub mod networks;

#[cfg(feature = "protocol")]
pub mod protocol;
#[cfg(feature = "subscription")]
pub mod subscription;
#[cfg(feature = "time")]
pub mod time;
#[cfg(feature = "validator")]
pub mod validator_info;

pub const IPV4_SUBNET_MASK: u8 = 24;
pub const IPV6_SUBNET_MASK: u8 = 96;
pub const PEER_COUNT_PER_IP_MAX: usize = 20;
pub const OUTBOUND_PEER_COUNT_PER_SUBNET_MAX: usize = 2;
pub const INBOUND_PEER_COUNT_PER_SUBNET_MAX: usize = 100;
pub const PEER_COUNT_MAX: usize = 4000;
pub const PEER_COUNT_DUMB_MAX: usize = 1000;
