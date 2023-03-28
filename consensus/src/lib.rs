#[macro_use]
extern crate log;

pub use consensus::consensus_proxy::ConsensusProxy;
pub use consensus::{Consensus, ConsensusEvent, RemoteEvent};
pub use error::{Error, SubscribeToAddressesError};

pub mod consensus;
pub mod error;
pub mod messages;
pub mod sync;
