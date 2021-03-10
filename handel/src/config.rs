use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    /// Number of peers contacted during an update at each level
    pub update_count: usize,

    /// Frequency at which updates are sent to peers
    pub update_interval: Duration,

    /// Timeout for levels
    pub timeout: Duration,

    /// How many peers are contacted at each level
    pub peer_count: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            update_count: 1,
            update_interval: Duration::from_millis(2000),
            timeout: Duration::from_millis(4000),
            peer_count: 2,
        }
    }
}
