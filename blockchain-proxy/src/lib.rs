//! This crate contains an abstraction over multiple types of blockchains.

pub use blockchain_proxy::{BlockchainProxy, BlockchainReadProxy};

pub(crate) mod blockchain_proxy;
