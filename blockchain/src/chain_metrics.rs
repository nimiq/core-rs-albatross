use std::sync::atomic::{AtomicUsize, Ordering};
use std::default::Default;
use crate::PushResult;

#[derive(Default)]
pub struct BlockchainMetrics {
    block_invalid_count: AtomicUsize,
    block_orphan_count: AtomicUsize,
    block_known_count: AtomicUsize,
    block_extended_count: AtomicUsize,
    block_rebranched_count: AtomicUsize,
    block_forked_count: AtomicUsize,
}

impl BlockchainMetrics {
    #[inline]
    pub fn note(&self, push_result: PushResult) {
        match push_result {
            PushResult::Invalid(_) => self.note_invalid_block(),
            PushResult::Orphan => self.note_orphan_block(),
            PushResult::Known => self.note_known_block(),
            PushResult::Extended => self.note_extended_block(),
            PushResult::Rebranched => self.note_rebranched_block(),
            PushResult::Forked => self.note_forked_block(),
        }
    }

    #[inline]
    pub fn note_invalid_block(&self) {
        self.block_invalid_count.fetch_add(1, Ordering::Release);
    }

    #[inline]
    pub fn block_invalid_count(&self) -> usize {
        self.block_invalid_count.load(Ordering::Acquire)
    }

    #[inline]
    pub fn note_orphan_block(&self) {
        self.block_orphan_count.fetch_add(1, Ordering::Release);
    }

    #[inline]
    pub fn block_orphan_count(&self) -> usize {
        self.block_orphan_count.load(Ordering::Acquire)
    }

    #[inline]
    pub fn note_known_block(&self) {
        self.block_known_count.fetch_add(1, Ordering::Release);
    }

    #[inline]
    pub fn block_known_count(&self) -> usize {
        self.block_known_count.load(Ordering::Acquire)
    }

    #[inline]
    pub fn note_extended_block(&self) {
        self.block_extended_count.fetch_add(1, Ordering::Release);
    }

    #[inline]
    pub fn block_extended_count(&self) -> usize {
        self.block_extended_count.load(Ordering::Acquire)
    }

    #[inline]
    pub fn note_rebranched_block(&self) {
        self.block_rebranched_count.fetch_add(1, Ordering::Release);
    }

    #[inline]
    pub fn block_rebranched_count(&self) -> usize {
        self.block_rebranched_count.load(Ordering::Acquire)
    }

    #[inline]
    pub fn note_forked_block(&self) {
        self.block_forked_count.fetch_add(1, Ordering::Release);
    }

    #[inline]
    pub fn block_forked_count(&self) -> usize {
        self.block_forked_count.load(Ordering::Acquire)
    }
}