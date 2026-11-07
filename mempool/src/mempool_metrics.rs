use prometheus_client::{
    encoding::{EncodeLabelSet, EncodeLabelValue},
    metrics::{counter::Counter, family::Family},
    registry::Registry,
};

use crate::mempool_state::EvictionReason;

#[derive(Default, Clone)]
pub struct MempoolMetrics {
    evicted_tx: Family<RemovedReasonLabel, Counter>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct RemovedReasonLabel {
    reason: TxRemovedReason,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
enum TxRemovedReason {
    BlockBuilding,
    Expired,
    AlreadyIncludedTx,
    Invalid,
    TooFull,
}

impl MempoolMetrics {
    pub fn register(&self, registry: &mut Registry) {
        registry.register(
            "removed_tx_count",
            "Number of transactions removed from mempool",
            self.evicted_tx.clone(),
        );
    }

    pub(crate) fn note_evicted(&self, reason: EvictionReason) {
        // Exhaustive on purpose: a new eviction reason must get a label instead of being
        // silently left out of the count.
        let reason = match reason {
            EvictionReason::BlockBuilding => TxRemovedReason::BlockBuilding,
            EvictionReason::Expired => TxRemovedReason::Expired,
            EvictionReason::AlreadyIncluded => TxRemovedReason::AlreadyIncludedTx,
            EvictionReason::Invalid => TxRemovedReason::Invalid,
            EvictionReason::TooFull => TxRemovedReason::TooFull,
        };
        self.evicted_tx
            .get_or_create(&RemovedReasonLabel { reason })
            .inc();
    }
}

#[cfg(test)]
mod tests {
    use prometheus_client::{encoding::text::encode, registry::Registry};

    use super::MempoolMetrics;
    use crate::mempool_state::EvictionReason;

    #[test]
    fn counts_transactions_removed_for_block_building() {
        let metrics = MempoolMetrics::default();
        let mut registry = Registry::default();
        metrics.register(&mut registry);

        metrics.note_evicted(EvictionReason::BlockBuilding);

        let mut encoded = String::new();
        encode(&mut encoded, &registry).unwrap();
        assert!(encoded.contains(r#"removed_tx_count_total{reason="BlockBuilding"} 1"#));
    }
}
