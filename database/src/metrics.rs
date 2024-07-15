// This file has been ported and adapted from reth (https://github.com/paradigmxyz/reth).
// Commit: 87cdfb185eaa721f18bc691ace87456fa348dbad
// License: MIT OR Apache-2.0

use std::{
    collections::HashMap,
    hash::BuildHasherDefault,
    time::{Duration, Instant},
};

use libmdbx::CommitLatency;
use log::warn;
use metrics::{counter, gauge, histogram, Counter, Gauge, Histogram, IntoLabels};
use parking_lot::RwLock;
use rustc_hash::{FxHashMap, FxHasher};
use strum::{EnumCount, IntoEnumIterator};
use strum_macros::{EnumCount as EnumCountMacro, EnumIter};

const LARGE_VALUE_THRESHOLD_BYTES: usize = 4096;

/// Caches metric handles for database environment to make sure handles are not re-created
/// on every operation.
///
/// Requires a metric recorder to be registered before creating an instance of this struct.
/// Otherwise, metric recording will no-op.
pub(crate) struct DatabaseEnvMetrics {
    /// Caches `OperationMetrics` handles for each table and operation tuple.
    operations: RwLock<FxHashMap<(String, Operation), OperationMetrics>>,
    /// Caches `TransactionMetrics` handles for counters grouped by only transaction mode.
    /// Updated both at tx open and close.
    transactions: FxHashMap<TransactionMode, TransactionMetrics>,
    /// Caches `TransactionOutcomeMetrics` handles for counters grouped by transaction mode and
    /// outcome. Can only be updated at tx close, as outcome is only known at that point.
    transaction_outcomes:
        FxHashMap<(TransactionMode, TransactionOutcome), TransactionOutcomeMetrics>,
}

impl DatabaseEnvMetrics {
    pub(crate) fn new() -> Self {
        // Pre-populate metric handle maps with all possible combinations of labels
        // to avoid runtime locks on the map when recording metrics.
        Self {
            operations: Default::default(),
            transactions: Self::generate_transaction_handles(),
            transaction_outcomes: Self::generate_transaction_outcome_handles(),
        }
    }

    /// Generate a map of all possible transaction modes to metric handles.
    /// Used for tracking a counter of open transactions.
    fn generate_transaction_handles() -> FxHashMap<TransactionMode, TransactionMetrics> {
        TransactionMode::iter()
            .map(|mode| {
                (
                    mode,
                    TransactionMetrics::new_with_labels(&[(
                        Labels::TransactionMode.as_str(),
                        mode.as_str(),
                    )]),
                )
            })
            .collect()
    }

    /// Generate a map of all possible transaction mode and outcome handles.
    /// Used for tracking various stats for finished transactions (e.g. commit duration).
    fn generate_transaction_outcome_handles(
    ) -> FxHashMap<(TransactionMode, TransactionOutcome), TransactionOutcomeMetrics> {
        let mut transaction_outcomes = HashMap::with_capacity_and_hasher(
            TransactionMode::COUNT * TransactionOutcome::COUNT,
            BuildHasherDefault::<FxHasher>::default(),
        );
        for mode in TransactionMode::iter() {
            for outcome in TransactionOutcome::iter() {
                transaction_outcomes.insert(
                    (mode, outcome),
                    TransactionOutcomeMetrics::new_with_labels(&[
                        (Labels::TransactionMode.as_str(), mode.as_str()),
                        (Labels::TransactionOutcome.as_str(), outcome.as_str()),
                    ]),
                );
            }
        }
        transaction_outcomes
    }

    /// Registers a new table for metrics recording.
    pub(crate) fn register_table(&self, table: &str) {
        for operation in Operation::iter() {
            self.operations.write().insert(
                (table.to_string(), operation),
                OperationMetrics::new_with_labels(&[
                    (Labels::Table.as_str(), table.to_string()),
                    (Labels::Operation.as_str(), operation.as_str().to_string()),
                ]),
            );
        }
    }

    /// Record a metric for database operation executed in `f`.
    pub(crate) fn record_operation<R>(
        &self,
        table: &str,
        operation: Operation,
        value_size: Option<usize>,
        f: impl FnOnce() -> R,
    ) -> R {
        if let Some(table_metrics) = self.operations.read().get(&(table.to_string(), operation)) {
            table_metrics.record(value_size, f)
        } else {
            warn!("no metric recorder found for table '{}'", table);
            f()
        }
    }

    /// Record metrics for opening a database transaction.
    pub(crate) fn record_opened_transaction(&self, mode: TransactionMode) {
        self.transactions
            .get(&mode)
            .expect("transaction mode metric handle not found")
            .record_open();
    }

    /// Record metrics for closing a database transactions.
    pub(crate) fn record_closed_transaction(
        &self,
        mode: TransactionMode,
        outcome: TransactionOutcome,
        open_duration: Duration,
        close_duration: Option<Duration>,
        commit_latency: Option<CommitLatency>,
    ) {
        self.transactions
            .get(&mode)
            .expect("transaction mode metric handle not found")
            .record_close();

        self.transaction_outcomes
            .get(&(mode, outcome))
            .expect("transaction outcome metric handle not found")
            .record(open_duration, close_duration, commit_latency);
    }
}

/// Transaction mode for the database, either read-only or read-write.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, EnumCountMacro, EnumIter)]
pub(crate) enum TransactionMode {
    /// Read-only transaction mode.
    ReadOnly,
    /// Read-write transaction mode.
    ReadWrite,
}

impl TransactionMode {
    /// Returns the transaction mode as a string.
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ReadWrite => "read-write",
        }
    }

    /// Returns `true` if the transaction mode is read-only.
    pub(crate) const fn is_read_only(&self) -> bool {
        matches!(self, Self::ReadOnly)
    }
}

/// Transaction outcome after a database operation - commit, abort, or drop.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, EnumCountMacro, EnumIter)]
pub(crate) enum TransactionOutcome {
    /// Successful commit of the transaction.
    Commit,
    /// Aborted transaction.
    Abort,
    /// Dropped transaction.
    Drop,
}

impl TransactionOutcome {
    /// Returns the transaction outcome as a string.
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Abort => "abort",
            Self::Drop => "drop",
        }
    }

    /// Returns `true` if the transaction outcome is a commit.
    pub(crate) const fn is_commit(&self) -> bool {
        matches!(self, Self::Commit)
    }
}

/// Types of operations conducted on the database: get, put, delete, and various cursor operations.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, EnumCountMacro, EnumIter)]
pub(crate) enum Operation {
    /// Database get operation.
    Get,
    /// Database put operation.
    Put,
    /// Database delete operation.
    Delete,
    /// Database clear operation.
    Clear,
    /// Database cursor delete current operation.
    CursorDeleteCurrent,
}

impl Operation {
    /// Returns the operation as a string.
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Put => "put",
            Self::Delete => "delete",
            Self::Clear => "clear",
            Self::CursorDeleteCurrent => "cursor-delete-current",
        }
    }
}

/// Enum defining labels for various aspects used in metrics.
enum Labels {
    /// Label representing a table.
    Table,
    /// Label representing a transaction mode.
    TransactionMode,
    /// Label representing a transaction outcome.
    TransactionOutcome,
    /// Label representing a database operation.
    Operation,
}

impl Labels {
    /// Converts each label variant into its corresponding string representation.
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::TransactionMode => "mode",
            Self::TransactionOutcome => "outcome",
            Self::Operation => "operation",
        }
    }
}

#[derive(Clone)]
pub(crate) struct TransactionMetrics {
    /// Total number of currently open database transactions
    open_total: Gauge,
}

impl TransactionMetrics {
    pub(crate) fn new_with_labels(labels: impl IntoLabels + Clone) -> Self {
        Self {
            open_total: gauge!("open_total", labels),
        }
    }

    pub(crate) fn record_open(&self) {
        self.open_total.increment(1.0);
    }

    pub(crate) fn record_close(&self) {
        self.open_total.decrement(1.0);
    }
}

#[derive(Clone)]
pub(crate) struct TransactionOutcomeMetrics {
    /// The time a database transaction has been open
    open_duration_seconds: Histogram,
    /// The time it took to close a database transaction
    close_duration_seconds: Histogram,
    /// The time it took to prepare a transaction commit
    commit_preparation_duration_seconds: Histogram,
    /// Duration of GC update during transaction commit by wall clock
    commit_gc_wallclock_duration_seconds: Histogram,
    /// The time it took to conduct audit of a transaction commit
    commit_audit_duration_seconds: Histogram,
    /// The time it took to write dirty/modified data pages to a filesystem during transaction
    /// commit
    commit_write_duration_seconds: Histogram,
    /// The time it took to sync written data to the disk/storage during transaction commit
    commit_sync_duration_seconds: Histogram,
    /// The time it took to release resources during transaction commit
    commit_ending_duration_seconds: Histogram,
    /// The total duration of a transaction commit
    commit_whole_duration_seconds: Histogram,
    /// User-mode CPU time spent on GC update during transaction commit
    commit_gc_cputime_duration_seconds: Histogram,
}

impl TransactionOutcomeMetrics {
    pub(crate) fn new_with_labels(labels: impl IntoLabels + Clone) -> Self {
        Self {
            open_duration_seconds: histogram!("open_duration_seconds", labels.clone()),
            close_duration_seconds: histogram!("close_duration_seconds", labels.clone()),
            commit_preparation_duration_seconds: histogram!(
                "commit_preparation_duration_seconds",
                labels.clone()
            ),
            commit_gc_wallclock_duration_seconds: histogram!(
                "commit_gc_wallclock_duration_seconds",
                labels.clone()
            ),
            commit_audit_duration_seconds: histogram!(
                "commit_audit_duration_seconds",
                labels.clone()
            ),
            commit_write_duration_seconds: histogram!(
                "commit_write_duration_seconds",
                labels.clone()
            ),
            commit_sync_duration_seconds: histogram!(
                "commit_sync_duration_seconds",
                labels.clone()
            ),
            commit_ending_duration_seconds: histogram!(
                "commit_ending_duration_seconds",
                labels.clone()
            ),
            commit_whole_duration_seconds: histogram!(
                "commit_whole_duration_seconds",
                labels.clone()
            ),
            commit_gc_cputime_duration_seconds: histogram!(
                "commit_gc_cputime_duration_seconds",
                labels
            ),
        }
    }

    /// Record transaction closing with the duration it was open and the duration it took to close
    /// it.
    pub(crate) fn record(
        &self,
        open_duration: Duration,
        close_duration: Option<Duration>,
        commit_latency: Option<CommitLatency>,
    ) {
        self.open_duration_seconds.record(open_duration);

        if let Some(close_duration) = close_duration {
            self.close_duration_seconds.record(close_duration)
        }

        if let Some(commit_latency) = commit_latency {
            self.commit_preparation_duration_seconds
                .record(commit_latency.preparation());
            self.commit_gc_wallclock_duration_seconds
                .record(commit_latency.gc_wallclock());
            self.commit_audit_duration_seconds
                .record(commit_latency.audit());
            self.commit_write_duration_seconds
                .record(commit_latency.write());
            self.commit_sync_duration_seconds
                .record(commit_latency.sync());
            self.commit_ending_duration_seconds
                .record(commit_latency.ending());
            self.commit_whole_duration_seconds
                .record(commit_latency.whole());
            self.commit_gc_cputime_duration_seconds
                .record(commit_latency.gc_cputime());
        }
    }
}

impl std::fmt::Debug for TransactionOutcomeMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransactionOutcomeMetrics").finish()
    }
}

#[derive(Clone)]
pub(crate) struct OperationMetrics {
    /// Total number of database operations made
    calls_total: Counter,
    /// The time it took to execute a database operation (`put/upsert/insert/append/append_dup`)
    /// with value larger than [`LARGE_VALUE_THRESHOLD_BYTES`] bytes.
    large_value_duration_seconds: Histogram,
}

impl OperationMetrics {
    pub(crate) fn new_with_labels(labels: impl IntoLabels + Clone) -> Self {
        Self {
            calls_total: counter!("calls_total", labels.clone()),
            large_value_duration_seconds: histogram!("large_value_duration_seconds", labels),
        }
    }

    /// Record operation metric.
    ///
    /// The duration it took to execute the closure is recorded only if the provided `value_size` is
    /// larger than [`LARGE_VALUE_THRESHOLD_BYTES`].
    pub(crate) fn record<R>(&self, value_size: Option<usize>, f: impl FnOnce() -> R) -> R {
        self.calls_total.increment(1);

        // Record duration only for large values to prevent the performance hit of clock syscall
        // on small operations
        if value_size.map_or(false, |size| size > LARGE_VALUE_THRESHOLD_BYTES) {
            let start = Instant::now();
            let result = f();
            self.large_value_duration_seconds.record(start.elapsed());
            result
        } else {
            f()
        }
    }
}

impl std::fmt::Debug for OperationMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationMetrics").finish()
    }
}
