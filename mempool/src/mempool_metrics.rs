use crate::mempool::EvictionReason;

#[derive(Default, Clone)]
pub struct MempoolMetrics {
    expired_tx: u64,
    already_included_tx: u64,
    invalid_tx: u64,
    too_full_tx: u64,
}

impl MempoolMetrics {
    pub(crate) fn note_evicted(&mut self, reason: EvictionReason) {
        match reason {
            EvictionReason::Expired => {
                self.expired_tx = self.expired_tx.wrapping_add(1);
            }
            EvictionReason::AlreadyIncluded => {
                self.already_included_tx = self.already_included_tx.wrapping_add(1);
            }
            EvictionReason::Invalid => {
                self.invalid_tx = self.invalid_tx.wrapping_add(1);
            }
            EvictionReason::TooFull => {
                self.too_full_tx = self.too_full_tx.wrapping_add(1);
            }
            _ => {}
        }
    }

    pub fn expired_tx_count(&self) -> u64 {
        self.expired_tx
    }

    pub fn already_included_tx_count(&self) -> u64 {
        self.already_included_tx
    }

    pub fn invalid_tx_count(&self) -> u64 {
        self.invalid_tx
    }

    pub fn too_full_tx_count(&self) -> u64 {
        self.too_full_tx
    }
}
