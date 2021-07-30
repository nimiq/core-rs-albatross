#![allow(dead_code)]
#![feature(map_first_last)]
#![feature(btree_drain_filter)]
#![feature(drain_filter)]
#![feature(hash_drain_filter)]

#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate futures;
#[macro_use]
extern crate log;
extern crate nimiq_block as block;
extern crate nimiq_blockchain as blockchain;
extern crate nimiq_collections as collections;
extern crate nimiq_database as database;
extern crate nimiq_hash as hash;
extern crate nimiq_macros as macros;
extern crate nimiq_mempool as mempool;
extern crate nimiq_messages as network_messages;
extern crate nimiq_network_interface as network_interface;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_utils as utils;
#[macro_use]
extern crate pin_project;

pub use consensus::{Consensus, ConsensusEvent, ConsensusProxy};
pub use error::Error;

pub mod consensus;
pub mod consensus_agent;
pub mod error;
pub mod messages;
pub mod sync;
