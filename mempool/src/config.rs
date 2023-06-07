use crate::{
    filter::{MempoolFilter, MempoolRules},
    mempool::Mempool,
};

/// Struct defining a Mempool configuration
#[derive(Debug, Clone)]
pub struct MempoolConfig {
    /// Total size limit of transactions in the regular mempool (bytes)
    pub size_limit: usize,
    /// Total size limit of transactions in the control mempool (bytes)
    pub control_size_limit: usize,
    /// Mempool filter rules
    pub filter_rules: MempoolRules,
    /// Mempool filter limit or size
    pub filter_limit: usize,
}

impl Default for MempoolConfig {
    fn default() -> MempoolConfig {
        MempoolConfig {
            size_limit: Mempool::DEFAULT_SIZE_LIMIT,
            control_size_limit: Mempool::DEFAULT_CONTROL_SIZE_LIMIT,
            filter_rules: MempoolRules::default(),
            filter_limit: MempoolFilter::DEFAULT_BLACKLIST_SIZE,
        }
    }
}
