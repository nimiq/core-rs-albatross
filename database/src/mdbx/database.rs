use std::{borrow::Cow, fs, path::Path, sync::Arc};

use libmdbx::NoWriteMap;
use log::info;

use super::{MdbxReadTransaction, MdbxWriteTransaction};
use crate::{traits::Database, DatabaseProxy, Error, TableFlags};

pub(super) type DbKvPair<'a> = (Cow<'a, [u8]>, Cow<'a, [u8]>);

/// Wrapper around the mdbx database handle.
/// A database can hold multiple tables.
#[derive(Clone, Debug)]
pub struct MdbxDatabase {
    pub(super) db: Arc<libmdbx::Database<NoWriteMap>>,
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

        MdbxTable { name }
    }

    fn read_transaction(&self) -> Self::ReadTransaction<'_> {
        MdbxReadTransaction::new(self.db.begin_ro_txn().unwrap())
    }

    fn write_transaction(&self) -> Self::WriteTransaction<'_> {
        MdbxWriteTransaction::new(self.db.begin_rw_txn().unwrap())
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

        let mdbx = MdbxDatabase { db: Arc::new(db) };
        if mdbx.need_resize(0) {
            info!("MDBX memory needs to be resized.");
        }

        Ok(mdbx)
    }

    pub fn need_resize(&self, threshold_size: usize) -> bool {
        let info = self.db.info().unwrap();
        let stat = self.db.stat().unwrap();

        let size_used = (stat.page_size() as usize) * (info.last_pgno() + 1);

        if threshold_size > 0 && info.map_size() - size_used < threshold_size {
            info!(
                size_used,
                threshold_size,
                map_size = info.map_size(),
                space_remaining = info.map_size() - size_used,
                "DB settings (threshold-based)"
            );
            return true;
        }

        // Resize is currently not supported. So don't let the resize happen
        // if a specific percentage is reached.
        let resize_percent: f64 = 1_f64;

        if (size_used as f64) / (info.map_size() as f64) > resize_percent {
            info!(
                map_size = info.map_size(),
                size_used,
                space_remaining = info.map_size() - size_used,
                percent_used = (size_used as f64) / (info.map_size() as f64),
                "DB resize (percent-based)"
            );
            return true;
        }

        false
    }
}

/// A table handle for the mdbx database.
/// It is used to reference tables during transactions.
#[derive(Debug)]
pub struct MdbxTable {
    pub(super) name: String,
}
