#![deny(missing_docs)]

//! Mempool implementation
//!
//! The mempool is the element inside the validator in charge of obtaining and
//! processing transactions. The validator will use the mempool to collect
//! transactions that should be included in a block.
#[macro_use]
extern crate log;
/// Mempool state module
mod mempool_state;

/// Mempool config module
pub mod config;
/// Mempool executor module
pub mod executor;

/// Mempool filter module
pub mod filter;
/// Main mempool module
pub mod mempool;
/// Mempool metrics
#[cfg(feature = "metrics")]
mod mempool_metrics;
/// Mempool transaction module
pub mod mempool_transactions;
/// Mempool syncer module
mod sync;
/// Verify transaction module
pub mod verify;
