use std::{borrow::Cow, fmt, fs, path::Path, sync::Arc};

use libmdbx::NoWriteMap;
use log::info;

use super::{MdbxReadTransaction, MdbxWriteTransaction};
use crate::{metrics::DatabaseEnvMetrics, traits::Database, DatabaseProxy, Error, TableFlags};

pub(super) type DbKvPair<'a> = (Cow<'a, [u8]>, Cow<'a, [u8]>);

/// Wrapper around the mdbx database handle.
/// A database can hold multiple tables.
#[derive(Clone)]
pub struct MdbxDatabase {
    pub(super) db: Arc<libmdbx::Database<NoWriteMap>>,
    pub(super) metrics: Option<Arc<DatabaseEnvMetrics>>,
}

impl fmt::Debug for MdbxDatabase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MdbxDatabase")
            .field("db", &self.db)
            .finish()
    }
}

impl Database for MdbxDatabase {
    type Table = MdbxTable;
    type ReadTransaction<'db> = MdbxReadTransaction<'db>
    where
        Self: 'db;
    type WriteTransaction<'db> = MdbxWriteTransaction<'db>
    where
        Self: 'db;

    fn open_table(&self, name: String) -> Self::Table {
        self.open_table_with_flags(name, TableFlags::empty())
    }

    fn open_table_with_flags(&self, name: String, flags: TableFlags) -> Self::Table {
        // This is an implicit transaction, so take the lock first.
        let mut table_flags = libmdbx::TableFlags::CREATE;

        // Translate flags.
        if flags.contains(TableFlags::DUPLICATE_KEYS) {
            table_flags.insert(libmdbx::TableFlags::DUP_SORT);

            if flags.contains(TableFlags::DUP_FIXED_SIZE_VALUES) {
                table_flags.insert(libmdbx::TableFlags::DUP_FIXED);
            }
        }
        if flags.contains(TableFlags::UINT_KEYS) {
            table_flags.insert(libmdbx::TableFlags::INTEGER_KEY);
        }

        // Create the database
        let txn = self.db.begin_rw_txn().unwrap();
        txn.create_table(Some(&name), table_flags).unwrap();
        txn.commit().unwrap();

        if let Some(ref metrics) = self.metrics {
            metrics.register_table(&name);
        }

        MdbxTable { name }
    }

    fn read_transaction(&self) -> Self::ReadTransaction<'_> {
        MdbxReadTransaction::new(self.db.begin_ro_txn().unwrap(), self.metrics.clone())
    }

    fn write_transaction(&self) -> Self::WriteTransaction<'_> {
        MdbxWriteTransaction::new(
            self.db.begin_rw_txn().unwrap(),
            self.metrics.as_ref().cloned(),
        )
    }
}

impl MdbxDatabase {
    #[allow(clippy::new_ret_no_self)]
    pub fn new<P: AsRef<Path>>(
        path: P,
        size: usize,
        max_tables: u32,
    ) -> Result<DatabaseProxy, Error> {
        Ok(DatabaseProxy::Persistent(MdbxDatabase::new_mdbx_database(
            path.as_ref(),
            size,
            max_tables,
            None,
        )?))
    }

    #[allow(clippy::new_ret_no_self)]
    pub fn new_with_max_readers<P: AsRef<Path>>(
        path: P,
        size: usize,
        max_tables: u32,
        max_readers: u32,
    ) -> Result<DatabaseProxy, Error> {
        Ok(DatabaseProxy::Persistent(MdbxDatabase::new_mdbx_database(
            path.as_ref(),
            size,
            max_tables,
            Some(max_readers),
        )?))
    }

    #[allow(clippy::new_ret_no_self)]
    pub fn new_with_metrics<P: AsRef<Path>>(
        path: P,
        size: usize,
        max_tables: u32,
    ) -> Result<DatabaseProxy, Error> {
        let mut db = MdbxDatabase::new_mdbx_database(path.as_ref(), size, max_tables, None)?;
        db.with_metrics();
        Ok(DatabaseProxy::Persistent(db))
    }

    #[allow(clippy::new_ret_no_self)]
    pub fn new_with_max_readers_and_metrics<P: AsRef<Path>>(
        path: P,
        size: usize,
        max_tables: u32,
        max_readers: u32,
    ) -> Result<DatabaseProxy, Error> {
        let mut db =
            MdbxDatabase::new_mdbx_database(path.as_ref(), size, max_tables, Some(max_readers))?;
        db.with_metrics();
        Ok(DatabaseProxy::Persistent(db))
    }

    pub(crate) fn new_mdbx_database(
        path: &Path,
        size: usize,
        max_tables: u32,
        max_readers: Option<u32>,
    ) -> Result<Self, Error> {
        fs::create_dir_all(path).map_err(Error::CreateDirectory)?;

        let db = libmdbx::Database::open_with_options(
            path,
            libmdbx::DatabaseOptions {
                max_tables: Some(max_tables.into()),
                max_readers,
                no_rdahead: true,
                mode: libmdbx::Mode::ReadWrite(libmdbx::ReadWriteOptions {
                    sync_mode: libmdbx::SyncMode::Durable, // default anyway
                    min_size: Some(0),
                    max_size: Some(size.try_into().unwrap()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )?;

        let info = db.info()?;
        let cur_mapsize = info.map_size();
        info!(cur_mapsize, "MDBX memory map size");

        let mdbx = MdbxDatabase {
            db: Arc::new(db),
            metrics: None,
        };

        Ok(mdbx)
    }

    pub(crate) fn with_metrics(&mut self) {
        self.metrics = Some(DatabaseEnvMetrics::new().into());
    }
}

/// A table handle for the mdbx database.
/// It is used to reference tables during transactions.
#[derive(Debug)]
pub struct MdbxTable {
    pub(super) name: String,
}
