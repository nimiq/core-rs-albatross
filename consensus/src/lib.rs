#[macro_use]
extern crate log;

pub use consensus::consensus_proxy::ConsensusProxy;
pub use consensus::{Consensus, ConsensusEvent, RemoteEvent};
pub use error::Error;
pub use error::SubscribeToAdressesError;

pub mod consensus;
pub mod error;
pub mod messages;
pub mod sync;
