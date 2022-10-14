#[macro_use]
extern crate log;

pub use consensus::consensus_proxy::ConsensusProxy;
pub use consensus::{Consensus, ConsensusEvent};
pub use error::Error;

pub mod consensus;
pub mod error;
pub mod messages;
pub mod sync;
