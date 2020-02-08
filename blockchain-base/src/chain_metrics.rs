use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{PushResult, PushError, BlockError};


#[derive(Default)]
pub struct BlockchainMetrics {
    block_invalid_count: AtomicUsize,
    block_orphan_count: AtomicUsize,
    block_known_count: AtomicUsize,
    block_extended_count: AtomicUsize,
    block_rebranched_count: AtomicUsize,
    block_forked_count: AtomicUsize,
    block_ignored_count: AtomicUsize,
}

impl BlockchainMetrics {
    #[inline]
    pub fn note<BE: BlockError>(&self, push_result: Result<PushResult, PushError<BE>>) {
        match push_result {
            Ok(PushResult::Known) => self.note_known_block(),
            Ok(PushResult::Extended) => self.note_extended_block(),
            Ok(PushResult::Rebranched) => self.note_rebranched_block(),
            Ok(PushResult::Forked) => self.note_forked_block(),
            Ok(PushResult::Ignored) => self.note_ignored_block(),
            Err(PushError::Orphan) => self.note_orphan_block(),
            Err(_) => self.note_invalid_block(),
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
    pub fn note_ignored_block(&self) {
        self.block_ignored_count.fetch_add(1, Ordering::Release);
    }

    #[inline]
    pub fn block_ignored_count(&self) -> usize {
        self.block_ignored_count.load(Ordering::Acquire)
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
