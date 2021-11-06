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
extern crate lazy_static;
#[macro_use]
extern crate log;
#[macro_use]
extern crate pin_project;

#[macro_use]
extern crate nimiq_macros;

pub use consensus::{Consensus, ConsensusEvent, ConsensusProxy};
pub use error::Error;

pub mod consensus;
pub mod consensus_agent;
pub mod error;
pub mod messages;
pub mod sync;
