#![feature(trait_alias)]

use nimiq_hash::Hash;
use std::fmt::Debug;

pub(crate) mod network;
pub(crate) mod outside_deps;
pub(crate) mod protocol;
pub(crate) mod state;
pub(crate) mod stream;
pub(crate) mod tendermint;
pub(crate) mod utils;

pub use outside_deps::TendermintOutsideDeps;
pub use state::TendermintState;
pub use stream::expect_block;
pub use tendermint::Tendermint;
pub use utils::*;

// These are trait aliases. We use them instead of repeating these trait bounds all throughout the
// code. It results in code that is cleaner and easier to understand.
pub trait ProposalTrait = Clone + Debug + PartialEq + Hash + Unpin;
pub trait ProofTrait = Clone + Debug + Unpin;
pub trait ResultTrait = Clone + Debug + Unpin;
