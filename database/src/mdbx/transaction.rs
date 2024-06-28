use std::{borrow::Cow, fmt, sync::Arc, time::Instant};

use libmdbx::{CommitLatency, NoWriteMap, TransactionKind, WriteFlags, RO, RW};
use log::debug;
use nimiq_database_value::{AsDatabaseBytes, FromDatabaseValue, IntoDatabaseValue};

use super::{MdbxCursor, MdbxTable, MdbxWriteCursor, MetricsHandler};
use crate::{
    metrics::{DatabaseEnvMetrics, Operation, TransactionOutcome},
    traits::{ReadTransaction, WriteTransaction},
};

/// Wrapper around mdbx transactions that only exposes our own traits.
pub struct MdbxTransaction<'db, K: TransactionKind> {
    txn: libmdbx::Transaction<'db, K, NoWriteMap>,
    metrics_handler: Option<MetricsHandler>,
}

impl<'db, K: TransactionKind> fmt::Debug for MdbxTransaction<'db, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MdbxTransaction")
            .field("txn", &self.txn)
            .finish()
    }
}

/// Instantiation for read-only transactions.
pub type MdbxReadTransaction<'db> = MdbxTransaction<'db, RO>;
/// Instantiation for read-write transactions.
pub type MdbxWriteTransaction<'db> = MdbxTransaction<'db, RW>;

impl<'db, Kind> MdbxTransaction<'db, Kind>
where
    Kind: TransactionKind,
{
    pub(crate) fn new(
        txn: libmdbx::Transaction<'db, Kind, NoWriteMap>,
        metrics: Option<Arc<DatabaseEnvMetrics>>,
    ) -> Self {
        MdbxTransaction {
            metrics_handler: metrics.map(|m| {
                let handler = MetricsHandler::new(txn.id(), m, Kind::ONLY_CLEAN);
                handler
                    .env_metrics
                    .record_opened_transaction(handler.transaction_mode());
                handler.log_transaction_opened();
                handler
            }),
            txn,
        }
    }

    pub(super) fn open_table(&self, table: &MdbxTable) -> libmdbx::Table {
        self.txn.open_table(Some(&table.name)).unwrap()
    }

    /// If `self.metrics_handler == Some(_)`, measure the time it takes to execute the closure and
    /// record a metric with the provided transaction outcome.
    ///
    /// Otherwise, just execute the closure.
    fn execute_with_close_transaction_metric(
        mut self,
        outcome: TransactionOutcome,
        f: impl FnOnce(Self) -> Option<CommitLatency>,
    ) {
        let run = |tx| {
            let start = Instant::now();
            let commit_latency = f(tx);
            let total_duration = start.elapsed();

            if outcome.is_commit() {
                debug!(
                    ?total_duration,
                    ?commit_latency,
                    is_read_only = Kind::ONLY_CLEAN,
                    "Commit"
                );
            }

            (commit_latency, total_duration)
        };

        if let Some(mut metrics_handler) = self.metrics_handler.take() {
            metrics_handler.set_close_recorded();
            metrics_handler.log_backtrace_on_long_read_transaction();

            let (commit_latency, close_duration) = run(self);
            metrics_handler.record_close(outcome, close_duration, commit_latency);
        } else {
            run(self);
        }
    }

    /// If `self.metrics_handler == Some(_)`, measure the time it takes to execute the closure and
    /// record a metric with the provided operation.
    ///
    /// Otherwise, just execute the closure.
    fn execute_with_operation_metric<'a, R>(
        &'a self,
        table_name: &str,
        operation: Operation,
        value_size: Option<usize>,
        f: impl FnOnce(&'a libmdbx::Transaction<'db, Kind, NoWriteMap>) -> R,
    ) -> R {
        if let Some(metrics_handler) = &self.metrics_handler {
            metrics_handler.log_backtrace_on_long_read_transaction();
            metrics_handler
                .env_metrics
                .record_operation(table_name, operation, value_size, || f(&self.txn))
        } else {
            f(&self.txn)
        }
    }
}

impl<'db, Kind> ReadTransaction<'db> for MdbxTransaction<'db, Kind>
where
    Kind: TransactionKind,
{
    type Table = MdbxTable;
    type Cursor<'txn> = MdbxCursor<'txn, Kind> where 'db: 'txn;

    fn get<K, V>(&self, table: &MdbxTable, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        let inner_table = self.open_table(table);

        let result: Option<Cow<[u8]>> =
            self.execute_with_operation_metric(&table.name, Operation::Get, None, |txn| {
                txn.get(&inner_table, &AsDatabaseBytes::as_database_bytes(key))
                    .unwrap()
            });

        Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
    }

    fn cursor<'txn>(&'txn self, table: &MdbxTable) -> MdbxCursor<'txn, Kind> {
        let inner_table = self.open_table(table);

        MdbxCursor::new(
            &table.name,
            self.txn.cursor(&inner_table).unwrap(),
            self.metrics_handler
                .as_ref()
                .map(|h| Arc::clone(&h.env_metrics)),
        )
    }
}

impl<'db> WriteTransaction<'db> for MdbxWriteTransaction<'db> {
    type WriteCursor<'txn> = MdbxWriteCursor<'txn> where 'db: 'txn;

    fn put_reserve<K, V>(&mut self, table: &MdbxTable, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: IntoDatabaseValue + ?Sized,
    {
        let inner_table = self.open_table(table);

        let key = AsDatabaseBytes::as_database_bytes(key);
        let value_size = IntoDatabaseValue::database_byte_size(value);

        let bytes: &mut [u8] = self.execute_with_operation_metric(
            &table.name,
            Operation::Put,
            Some(value_size),
            |txn| {
                txn.reserve(&inner_table, key, value_size, WriteFlags::empty())
                    .unwrap()
            },
        );

        IntoDatabaseValue::copy_into_database(value, bytes);
    }

    fn put<K, V>(&mut self, table: &MdbxTable, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let inner_table = self.open_table(table);

        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);

        self.execute_with_operation_metric(&table.name, Operation::Put, Some(value.len()), |txn| {
            txn.put(&inner_table, key, value, WriteFlags::empty())
                .unwrap()
        });
    }

    fn append<K, V>(&mut self, table: &MdbxTable, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let table = self.open_table(table);

        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);

        self.txn
            .put(&table, key, value, WriteFlags::APPEND)
            .unwrap();
    }

    fn remove<K>(&mut self, table: &MdbxTable, key: &K)
    where
        K: AsDatabaseBytes + ?Sized,
    {
        let inner_table = self.open_table(table);

        self.execute_with_operation_metric(&table.name, Operation::Delete, None, |txn| {
            txn.del(
                &inner_table,
                AsDatabaseBytes::as_database_bytes(key).as_ref(),
                None,
            )
            .unwrap()
        });
    }

    fn remove_item<K, V>(&mut self, table: &MdbxTable, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let inner_table = self.open_table(table);

        self.execute_with_operation_metric(&table.name, Operation::Delete, None, |txn| {
            txn.del(
                &inner_table,
                AsDatabaseBytes::as_database_bytes(key).as_ref(),
                Some(AsDatabaseBytes::as_database_bytes(value).as_ref()),
            )
            .unwrap()
        });
    }

    fn commit(self) {
        self.execute_with_close_transaction_metric(TransactionOutcome::Commit, |this| {
            let (_, latency, _) = this.txn.commit_and_rebind_open_dbs_with_latency().unwrap();
            Some(latency)
        });
    }

    fn abort(self) {
        self.execute_with_close_transaction_metric(TransactionOutcome::Abort, |this| {
            drop(this.txn);
            None
        })
    }

    fn cursor<'txn>(&'txn self, table: &MdbxTable) -> MdbxWriteCursor<'txn> {
        let inner_table = self.open_table(table);

        MdbxWriteCursor::new(
            &table.name,
            self.txn.cursor(&inner_table).unwrap(),
            self.metrics_handler
                .as_ref()
                .map(|h| Arc::clone(&h.env_metrics)),
        )
    }

    fn clear_database(&mut self, table: &MdbxTable) {
        let inner_table = self.open_table(table);

        self.execute_with_operation_metric(&table.name, Operation::Clear, None, |txn| {
            txn.clear_table(&inner_table).unwrap()
        });
    }
}
