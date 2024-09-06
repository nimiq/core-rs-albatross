use std::time::Duration;

/// Handel configuration settings
#[derive(Clone, Debug)]
pub struct Config {
    /// Frequency at which updates are sent to peers.
    pub update_interval: Duration,

    /// Maximum time to wait for a level to complete before starting the next level.
    pub level_timeout: Duration,

    /// Number of peers that are contacted at each level.
    pub peer_count: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            update_interval: Duration::from_millis(500),
            level_timeout: Duration::from_millis(400),
            peer_count: 2,
        }
    }
}
