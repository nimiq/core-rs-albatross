pub mod block_live_sync;
pub mod block_queue;
pub mod block_request_component;
pub mod chunk_request_component;
pub mod state_queue;

pub use block_live_sync::BlockLiveSync;
use nimiq_primitives::policy::Policy;

#[derive(Clone, Debug)]
pub struct BlockQueueConfig {
    /// Buffer size limit
    pub buffer_max: usize,

    /// How many blocks ahead we will buffer.
    pub window_ahead_max: u32,

    /// How many blocks back into the past we tolerate without returning a peer as Outdated.
    pub tolerate_past_max: u32,

    /// Flag to indicate if micro blocks should carry a body
    pub include_micro_bodies: bool,
}

impl Default for BlockQueueConfig {
    fn default() -> Self {
        Self {
            buffer_max: 4 * Policy::blocks_per_batch() as usize,
            window_ahead_max: 2 * Policy::blocks_per_batch(),
            tolerate_past_max: Policy::blocks_per_batch(),
            include_micro_bodies: true,
        }
    }
}
