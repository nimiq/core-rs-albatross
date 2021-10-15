#![feature(total_cmp)]

//! Mempool implementation
//!
//! The mempool is the element inside the validator in charge of obtaining and
//! processing transactions. The validator will use the mempool to collect
//! transactions that should be included in a block.

extern crate log;
/// Mempool config module
pub mod config;
/// Mempool executor module
pub mod executor;
/// Mempool filter module
pub mod filter;
/// Main mempool module
pub mod mempool;
