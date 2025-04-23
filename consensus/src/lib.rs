//! The `consensus` crate implements the Albatross consensus protocol.
//! It coordinates synchronization and peer-to-peer communication.
//!
//! The main modules are:
//! - `consensus`: manages the consensus state.
//! - `sync`: handles synchronization across different modes.
//! - `messages`: defines the network message formats.
//! - `error`: defines errors related to consensus and communication.
//! - `bls_cache`: internal LRU cache for optimizing BLS key usage.

#[macro_use]
extern crate log;

pub use bls_cache::BlsCache;
pub use consensus::{consensus_proxy::ConsensusProxy, Consensus, ConsensusEvent, RemoteEvent};
pub use error::{Error, SubscribeToAddressesError};

mod bls_cache;

pub mod consensus;
pub mod error;
pub mod messages;
pub mod sync;
