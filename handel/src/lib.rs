#![recursion_limit = "256"]
// unfortunately necessary for the rather big select! macro in aggregation.rs > Aggregation::next
extern crate beserial;
#[macro_use]
extern crate beserial_derive;
/// Handel implementation for Nimiq's Rust Albatross client.
///
/// Handel[1] is byzantine fault-tolerant signature aggregation protocol. Albatross uses Handel to
/// aggregate signatures for view changes and the pBFT prepare and commit phases.
///
/// [1] [Handel: Practical Multi-Signature Aggregation for Large Byzantine Committees](https://arxiv.org/abs/1906.05132)
extern crate futures;
extern crate futures_cpupool;
#[macro_use]
extern crate log;
extern crate nimiq_bls as bls;
extern crate nimiq_collections as collections;
extern crate nimiq_hash as hash;
extern crate nimiq_macros as macros;
extern crate nimiq_utils as utils;
extern crate parking_lot;
extern crate rand;
extern crate tokio;

pub mod aggregation;
pub mod config;
pub mod contribution;
pub mod evaluator;
pub mod identity;
pub mod level;
pub mod partitioner;
pub mod protocol;
pub mod store;
pub(crate) mod todo;
pub mod update;
pub mod verifier;
