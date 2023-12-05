mod sync;
mod sync_requests;
mod sync_stream;
mod validity_window;

use nimiq_primitives::policy::Policy;
pub use sync::LightMacroSync;

/// Minimum distance to light sync in #blocks from the peers head.
pub fn full_sync_threshold() -> u32 {
    // TODO: Experimental value that should be improved based on data collected from the testnet.
    Policy::blocks_per_epoch() / 4
}
