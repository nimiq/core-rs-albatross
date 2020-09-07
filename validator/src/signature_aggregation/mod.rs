// pub mod pbft;
mod registry;
/// Implementation of signature aggregation protocols (view change and pBFT prepare/commit) using
/// the Handel protocol. The Handel protocol itself is implemented in the nimiq-handel crate.
mod verifier;
pub mod view_change;
// pub mod voting;
