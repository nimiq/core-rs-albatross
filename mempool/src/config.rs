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
    /// Maximum number of transactions stored per sender
    pub tx_per_sender_limit: usize,
    /// Maximum number of transactions verified at the same time. Verification runs on blocking
    /// threads, so this is a CPU budget for transaction verification, not a thread count.
    pub max_concurrent_verifications: usize,
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
            tx_per_sender_limit: Mempool::DEFAULT_TX_PER_SENDER_LIMIT,
            max_concurrent_verifications: Mempool::default_max_concurrent_verifications(),
            filter_rules: MempoolRules::default(),
            filter_limit: MempoolFilter::DEFAULT_BLACKLIST_SIZE,
        }
    }
}
