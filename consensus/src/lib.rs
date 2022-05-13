#[macro_use]
extern crate log;

pub use consensus::{Consensus, ConsensusEvent, ConsensusProxy};
pub use error::Error;

pub mod consensus;
pub mod error;
pub mod messages;
pub mod sync;
