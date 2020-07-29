use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    /// Number of peers contacted during an update at each level
    pub update_count: usize,

    /// Frequency at which updates are sent to peers
    pub update_interval: Duration,

    /// Grace period handel will aggregate additional signatures once the threshold is reached
    pub grace_period: Duration,

    /// Timeout for levels
    pub timeout: Duration,

    /// How many peers are contacted at each level
    pub peer_count: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            update_count: 1,
            update_interval: Duration::from_millis(100),
            timeout: Duration::from_millis(500),
            grace_period: Duration::from_millis(50),
            peer_count: 10,
        }
    }
}
