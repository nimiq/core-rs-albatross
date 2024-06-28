// This file has been ported and adapted from reth (https://github.com/paradigmxyz/reth).
// Commit: 87cdfb185eaa721f18bc691ace87456fa348dbad
// License: MIT OR Apache-2.0

use std::{
    backtrace::Backtrace,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use libmdbx::CommitLatency;
use log::{trace, warn};

use crate::metrics::{DatabaseEnvMetrics, TransactionMode, TransactionOutcome};

/// Duration after which a transaction is considered long-running and its backtrace is logged.
const LONG_TRANSACTION_DURATION: Duration = Duration::from_secs(60);

pub struct MetricsHandler {
    /// Some identifier of transaction.
    txn_id: u64,
    /// The time when transaction was opened.
    start: Instant,
    /// If `true`, the metric about transaction closing has already been recorded and we don't need
    /// to do anything on [`Drop::drop`].
    close_recorded: bool,
    /// If `true`, the backtrace of transaction will be recorded and logged.
    /// See [`MetricsHandler::log_backtrace_on_long_read_transaction`].
    record_backtrace: bool,
    /// If `true`, the backtrace of transaction has already been recorded and logged.
    /// See [`MetricsHandler::log_backtrace_on_long_read_transaction`].
    backtrace_recorded: AtomicBool,
    pub(super) env_metrics: Arc<DatabaseEnvMetrics>,
    transaction_mode: TransactionMode,
}

impl MetricsHandler {
    pub(super) fn new(txn_id: u64, env_metrics: Arc<DatabaseEnvMetrics>, read_only: bool) -> Self {
        Self {
            txn_id,
            start: Instant::now(),
            close_recorded: false,
            record_backtrace: true,
            backtrace_recorded: AtomicBool::new(false),
            env_metrics,
            transaction_mode: if read_only {
                TransactionMode::ReadOnly
            } else {
                TransactionMode::ReadWrite
            },
        }
    }

    pub(super) const fn transaction_mode(&self) -> TransactionMode {
        self.transaction_mode
    }

    /// Logs the caller location and ID of the transaction that was opened.
    #[track_caller]
    pub(super) fn log_transaction_opened(&self) {
        trace!(
            caller = %core::panic::Location::caller(),
            id = %self.txn_id,
            read_only = %self.transaction_mode().is_read_only(),
            "Transaction opened",
        );
    }

    /// Logs the backtrace of current call if the duration that the read transaction has been open
    /// is more than [`LONG_TRANSACTION_DURATION`] and `record_backtrace == true`.
    /// The backtrace is recorded and logged just once, guaranteed by `backtrace_recorded` atomic.
    ///
    /// NOTE: Backtrace is recorded using [`Backtrace::force_capture`], so `RUST_BACKTRACE` env var
    /// is not needed.
    pub(super) fn log_backtrace_on_long_read_transaction(&self) {
        if self.record_backtrace
            && !self.backtrace_recorded.load(Ordering::Relaxed)
            && self.transaction_mode().is_read_only()
        {
            let open_duration = self.start.elapsed();
            if open_duration >= LONG_TRANSACTION_DURATION {
                self.backtrace_recorded.store(true, Ordering::Relaxed);
                warn!(
                    target: "storage::db::mdbx",
                    ?open_duration,
                    %self.txn_id,
                    "The database read transaction has been open for too long. Backtrace:\n{}", Backtrace::force_capture()
                );
            }
        }
    }

    #[inline]
    pub(super) fn set_close_recorded(&mut self) {
        self.close_recorded = true;
    }

    #[inline]
    pub(super) fn record_close(
        &mut self,
        outcome: TransactionOutcome,
        close_duration: Duration,
        commit_latency: Option<CommitLatency>,
    ) {
        let open_duration = self.start.elapsed();
        self.env_metrics.record_closed_transaction(
            self.transaction_mode(),
            outcome,
            open_duration,
            Some(close_duration),
            commit_latency,
        );
    }
}

impl Drop for MetricsHandler {
    fn drop(&mut self) {
        if !self.close_recorded {
            self.log_backtrace_on_long_read_transaction();
            self.env_metrics.record_closed_transaction(
                self.transaction_mode(),
                TransactionOutcome::Drop,
                self.start.elapsed(),
                None,
                None,
            );
        }
    }
}
