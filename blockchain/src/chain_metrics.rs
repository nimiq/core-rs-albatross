use crate::{PushError, PushResult};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct BlockchainMetrics {
    block_invalid_count: AtomicU64,
    block_orphan_count: u64,
    block_known_count: u64,
    block_extended_count: u64,
    block_rebranched_count: u64,
    block_forked_count: u64,
    block_ignored_count: u64,
}

impl BlockchainMetrics {
    #[inline]
    pub fn note<BE>(&mut self, push_result: Result<PushResult, PushError>) {
        match push_result {
            Ok(PushResult::Known) => {
                self.block_known_count = self.block_known_count.wrapping_add(1)
            }
            Ok(PushResult::Extended) => {
                self.block_extended_count = self.block_extended_count.wrapping_add(1);
            }
            Ok(PushResult::Rebranched) => {
                self.block_rebranched_count = self.block_rebranched_count.wrapping_add(1);
            }
            Ok(PushResult::Forked) => {
                self.block_forked_count = self.block_forked_count.wrapping_add(1);
            }
            Ok(PushResult::Ignored) => {
                self.block_ignored_count = self.block_ignored_count.wrapping_add(1);
            }
            Err(PushError::Orphan) => {
                self.block_orphan_count = self.block_orphan_count.wrapping_add(1);
            }
            Err(_) => self.note_invalid_block(),
        };
    }

    #[inline]
    pub fn note_invalid_block(&self) {
        self.block_invalid_count.fetch_add(1, Ordering::Release);
    }

    #[inline]
    pub fn block_invalid_count(&self) -> u64 {
        self.block_invalid_count.load(Ordering::Acquire)
    }

    #[inline]
    pub fn block_orphan_count(&self) -> u64 {
        self.block_orphan_count
    }

    #[inline]
    pub fn block_known_count(&self) -> u64 {
        self.block_known_count
    }

    #[inline]
    pub fn block_extended_count(&self) -> u64 {
        self.block_extended_count
    }

    #[inline]
    pub fn block_rebranched_count(&self) -> u64 {
        self.block_rebranched_count
    }

    #[inline]
    pub fn block_ignored_count(&self) -> u64 {
        self.block_ignored_count
    }

    #[inline]
    pub fn block_forked_count(&self) -> u64 {
        self.block_forked_count
    }
}
