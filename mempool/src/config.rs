use crate::filter::{MempoolFilter, MempoolRules};

#[derive(Debug, Clone)]
pub struct MempoolConfig {
    pub filter_rules: MempoolRules,
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
