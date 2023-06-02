use std::{borrow::Cow, fs, path::Path, sync::Arc};

use libmdbx::NoWriteMap;
use log::info;

use crate::{traits::Database, DatabaseProxy, Error, TableFlags};

use super::{MdbxReadTransaction, MdbxWriteTransaction};

pub(super) type DbKvPair<'a> = (Cow<'a, [u8]>, Cow<'a, [u8]>);

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

    fn read_transaction<'db>(&'db self) -> Self::ReadTransaction<'db> {
        MdbxReadTransaction::new(self.db.begin_ro_txn().unwrap())
    }

    fn write_transaction<'db>(&'db self) -> Self::WriteTransaction<'db> {
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

        let mut db = libmdbx::Database::new();

        // Configure the environment flags
        let geo = libmdbx::Geometry::<std::ops::Range<usize>> {
            size: Some(0..size),
            ..Default::default()
        };

        db.set_geometry(geo);

        let db_flags = libmdbx::DatabaseFlags {
            no_rdahead: true,
            mode: libmdbx::Mode::ReadWrite {
                sync_mode: libmdbx::SyncMode::Durable,
            },
            ..Default::default()
        };

        db.set_flags(db_flags);

        // This is only required if multiple databases will be used in the environment.
        db.set_max_tables(max_tables as usize);
        if let Some(max_readers) = max_readers {
            db.set_max_readers(max_readers);
        }

        let db = db.open(path)?;

        let info = db.info()?;
        let cur_mapsize = info.map_size();
        info!("MDBX memory map size: {}", cur_mapsize);

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
            info!("DB resize (threshold-based)");
            info!("DB map size: {}", info.map_size());
            info!("Space used: {}", size_used);
            info!("Space remaining: {}", info.map_size() - size_used);
            info!("Size threshold: {}", threshold_size);
            return true;
        }

        // Resize is currently not supported. So don't let the resize happen
        // if a specific percentage is reached.
        let resize_percent: f64 = 1_f64;

        if (size_used as f64) / (info.map_size() as f64) > resize_percent {
            info!("DB resize (percent-based)");
            info!("DB map size: {}", info.map_size());
            info!("Space used: {}", size_used);
            info!("Space remaining: {}", info.map_size() - size_used);
            info!(
                "Percent used: {:.2}",
                (size_used as f64) / (info.map_size() as f64)
            );
            return true;
        }

        false
    }
}

#[derive(Debug)]
pub struct MdbxTable {
    pub(super) name: String,
}
