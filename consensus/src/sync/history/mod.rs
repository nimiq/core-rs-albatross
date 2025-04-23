//! Handles the logic of syncing for history nodes.

pub mod cluster;
mod sync;
mod sync_clustering;
mod sync_stream;

pub use sync::HistoryMacroSync;
