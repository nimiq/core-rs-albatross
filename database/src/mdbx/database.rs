use std::{any::TypeId, fs, ops::Range, path::Path, sync::Arc};

use libmdbx::NoWriteMap;
use log::{debug, info};
use tempfile::TempDir;

use super::{MdbxReadTransaction, MdbxWriteTransaction};
use crate::{
    traits::{AsDatabaseBytes, Database, DupTable, RegularTable, Table},
    Error,
};

const GIGABYTE: usize = 1024 * 1024 * 1024;
const TERABYTE: usize = GIGABYTE * 1024;

/// Database config options.
pub struct DatabaseConfig {
    /// The maximum number of tables that can be opened.
    pub max_tables: Option<u64>,
    /// The maximum number of reader slots.
    pub max_readers: Option<u32>,
    /// Whether to enable/disable readahead.
    pub no_rdahead: bool,
    /// The minimum/maximum file size of the database. Default max size is 2TB.
    pub size: Option<Range<isize>>,
    /// Aims to coalesce a Garbage Collection items.
    pub coalesce: bool,
    /// The growth step by which the database file will be increased when lacking space.
    pub growth_step: Option<isize>,
    /// The threshold of unused space, after which the database file will be shrunk.
    pub shrink_threshold: Option<isize>,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        DatabaseConfig {
            max_tables: Some(20),
            max_readers: None,
            no_rdahead: true,
            // Default max database size: 2TB
            size: Some(0..(2 * TERABYTE as isize)),
            coalesce: false,
            // Default growth step: 4GB
            growth_step: Some(4 * GIGABYTE as isize),
            shrink_threshold: None,
        }
    }
}

impl From<DatabaseConfig> for libmdbx::DatabaseOptions {
    fn from(value: DatabaseConfig) -> Self {
        libmdbx::DatabaseOptions {
            max_tables: value.max_tables,
            max_readers: value.max_readers,
            no_rdahead: value.no_rdahead,
            mode: libmdbx::Mode::ReadWrite(libmdbx::ReadWriteOptions {
                sync_mode: libmdbx::SyncMode::Durable,
                min_size: value.size.as_ref().map(|r| r.start),
                max_size: value.size.map(|r| r.end),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

/// Wrapper around the mdbx database handle.
/// A database can hold multiple tables.
#[derive(Clone, Debug)]
pub struct MdbxDatabase {
    /// The database handle.
    db: Arc<libmdbx::Database<NoWriteMap>>,
    /// For volatile databases, this is the temporary directory handle,
    /// which will clean up on `Drop`.
    temp_dir: Option<Arc<TempDir>>,
}

impl MdbxDatabase {
    /// Create a table with additional flags.
    fn create_table<T: Table>(&self, _table: &T, mut flags: libmdbx::TableFlags) {
        // Ensure `CREATE` flag is set.
        flags.insert(libmdbx::TableFlags::CREATE);

        // Automatically set the integer key flag.
        let key_type = TypeId::of::<T::Key>();
        if key_type == TypeId::of::<u32>() || key_type == TypeId::of::<u64>() {
            flags.insert(libmdbx::TableFlags::INTEGER_KEY);
        }

        // Create the table with an implicit transaction.
        let txn = self.db.begin_rw_txn().unwrap();
        debug!("Creating table: {}, flags: {:?}", T::NAME, flags);
        txn.create_table(Some(T::NAME), flags).unwrap();
        txn.commit().unwrap();
    }

    /// Creates a new database at the given path.
    pub fn new<P: AsRef<Path>>(path: P, config: DatabaseConfig) -> Result<Self, Error> {
        fs::create_dir_all(path.as_ref()).map_err(Error::CreateDirectory)?;

        let db =
            libmdbx::Database::open_with_options(path, libmdbx::DatabaseOptions::from(config))?;

        let info = db.info()?;
        let cur_mapsize = info.map_size();
        info!(cur_mapsize, "MDBX memory map size");

        let mdbx = MdbxDatabase {
            db: Arc::new(db),
            temp_dir: None,
        };

        Ok(mdbx)
    }

    /// Creates a volatile database (in a temporary directory, which cleans itself after use).
    pub fn new_volatile(config: DatabaseConfig) -> Result<Self, Error> {
        let temp_dir = Arc::new(TempDir::new()?);
        let mut mdbx = MdbxDatabase::new(temp_dir.path(), config)?;
        mdbx.temp_dir = Some(temp_dir);

        Ok(mdbx)
    }
}

impl Database for MdbxDatabase {
    type ReadTransaction<'db> = MdbxReadTransaction<'db>;

    type WriteTransaction<'db> = MdbxWriteTransaction<'db>;

    /// Creates a regular table (no-duplicates).
    fn create_regular_table<T: RegularTable>(&self, table: &T) {
        self.create_table(table, libmdbx::TableFlags::empty())
    }

    /// Creates a regular table (no-duplicates).
    fn create_dup_table<T: DupTable>(&self, table: &T) {
        let mut dup_flags = libmdbx::TableFlags::DUP_SORT;

        // Set the fixed size flag if given.
        if T::Value::FIXED_SIZE.is_some() {
            dup_flags.insert(libmdbx::TableFlags::DUP_FIXED);
        }

        self.create_table(table, dup_flags)
    }

    fn read_transaction(&self) -> Self::ReadTransaction<'_> {
        MdbxReadTransaction::new_read(self.db.begin_ro_txn().unwrap())
    }

    fn write_transaction(&self) -> Self::WriteTransaction<'_> {
        MdbxWriteTransaction::new(self.db.begin_rw_txn().unwrap())
    }
}
