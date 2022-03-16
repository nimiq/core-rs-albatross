#[macro_use]
extern crate log;

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
pub use stream::TendermintStreamWrapper as Tendermint;
pub use utils::*;

// Poor man's trait aliases:

pub trait ProposalTrait: Clone + Debug + PartialEq + Unpin + Send + Sync + 'static {}
impl<T: Clone + Debug + PartialEq + Unpin + Send + Sync + 'static> ProposalTrait for T {}

pub trait ProposalHashTrait: Clone + Debug + PartialEq + Ord + Unpin + Send + 'static {}
impl<T: Clone + Debug + PartialEq + Ord + Unpin + Send + 'static> ProposalHashTrait for T {}

pub trait ProofTrait: Clone + Debug + Unpin + Send + 'static {}
impl<T: Clone + Debug + Unpin + Send + 'static> ProofTrait for T {}

pub trait ResultTrait: Clone + Debug + Unpin + Send + 'static {}
impl<T: Clone + Debug + Unpin + Send + 'static> ResultTrait for T {}
