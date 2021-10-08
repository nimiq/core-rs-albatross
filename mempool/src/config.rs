use crate::filter::{MempoolFilter, MempoolRules};

/// Struct defining a Mempool configuration
#[derive(Debug, Clone)]
pub struct MempoolConfig {
    /// Mempool filter rules
    pub filter_rules: MempoolRules,
    /// Mempool filter limit or size
    pub filter_limit: usize,
}

impl Default for MempoolConfig {
    fn default() -> MempoolConfig {
        MempoolConfig {
            filter_rules: MempoolRules::default(),
            filter_limit: MempoolFilter::DEFAULT_BLACKLIST_SIZE,
        }
    }
}
