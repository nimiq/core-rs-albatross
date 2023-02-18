use crate::mempool_state::EvictionReason;
use prometheus_client::{
    encoding::{EncodeLabelSet, EncodeLabelValue},
    metrics::{counter::Counter, family::Family},
    registry::Registry,
};

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
        let reason = match reason {
            EvictionReason::Expired => TxRemovedReason::Expired,
            EvictionReason::AlreadyIncluded => TxRemovedReason::AlreadyIncludedTx,
            EvictionReason::Invalid => TxRemovedReason::Invalid,
            EvictionReason::TooFull => TxRemovedReason::TooFull,
            _ => return,
        };
        self.evicted_tx
            .get_or_create(&RemovedReasonLabel { reason })
            .inc();
    }
}
