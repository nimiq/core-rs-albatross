/// Handel implementation for Nimiq's Rust Albatross client.
///
/// Handel[1] is byzantine fault-tolerant signature aggregation protocol. Albatross uses Handel to
/// aggregate signatures for view changes and the pBFT prepare and commit phases.
///
/// [1] [Handel: Practical Multi-Signature Aggregation for Large Byzantine Committees](https://arxiv.org/abs/1906.05132)

extern crate futures;
extern crate stopwatch;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate log;
extern crate tokio;
extern crate rand;
extern crate parking_lot;

extern crate beserial;
#[macro_use]
extern crate beserial_derive;
extern crate nimiq_bls as bls;
extern crate nimiq_collections as collections;
extern crate nimiq_hash as hash;
extern crate nimiq_utils as utils;
extern crate nimiq_macros as macros;


pub mod multisig;
pub mod verifier;
pub mod store;
pub mod evaluator;
pub mod identity;
pub mod partitioner;
pub mod timeout;
pub mod config;
pub mod level;
pub mod protocol;
pub mod update;
pub mod aggregation;
pub mod sender;
mod todo;
