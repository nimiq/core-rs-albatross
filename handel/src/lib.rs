/// Handel implementation for Nimiq's Rust Albatross client.
///
/// Handel[1] is byzantine fault-tolerant signature aggregation protocol. Albatross uses Handel to
/// aggregate signatures for skip blocks and the pBFT prepare and commit phases.
///
/// [1] [Handel: Practical Multi-Signature Aggregation for Large Byzantine Committees](https://arxiv.org/abs/1906.05132)
#[macro_use]
extern crate log;

pub mod aggregation;
pub mod config;
pub mod contribution;
pub mod evaluator;
pub mod identity;
pub mod level;
pub mod network;
pub mod partitioner;
pub mod protocol;
pub mod store;
pub(crate) mod todo;
pub mod update;
pub mod verifier;
