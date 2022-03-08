use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Arc;

pub use libmdbx::Error as LmdbError;
use libmdbx::{NoWriteMap, Transaction, WriteFlags, RO, RW};

use crate::cursor::{RawReadCursor, ReadCursor, WriteCursor as WriteCursorTrait};

use super::*;

type DbKvPair<'a> = (Cow<'a, [u8]>, Cow<'a, [u8]>);

#[derive(Debug)]
pub struct MdbxEnvironment {
    env: Arc<libmdbx::Environment<NoWriteMap>>,
    path: String,
}

impl Clone for MdbxEnvironment {
    fn clone(&self) -> Self {
        Self {
            env: Arc::clone(&self.env),
            path: self.path.clone(),
        }
    }
}

impl MdbxEnvironment {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(path: &str, size: usize, max_dbs: u32) -> Result<Environment, LmdbError> {
        Ok(Environment::Persistent(
            MdbxEnvironment::new_mdbx_environment(path, size, max_dbs, None)?,
        ))
    }

    #[allow(clippy::new_ret_no_self)]
    pub fn new_with_max_readers(
        path: &str,
        size: usize,
        max_dbs: u32,
        max_readers: u32,
    ) -> Result<Environment, LmdbError> {
        Ok(Environment::Persistent(
            MdbxEnvironment::new_mdbx_environment(path, size, max_dbs, Some(max_readers))?,
        ))
    }

    pub(super) fn new_mdbx_environment(
        path: &str,
        size: usize,
        max_dbs: u32,
        max_readers: Option<u32>,
    ) -> Result<Self, LmdbError> {
        fs::create_dir_all(path).unwrap();

        let mut env = libmdbx::Environment::new();

        //Configure the environment flags
        let geo = libmdbx::Geometry::<std::ops::Range<usize>> {
            size: Some(0..size),
            ..Default::default()
        };

        env.set_geometry(geo);

        let db_flags = libmdbx::EnvironmentFlags {
            no_rdahead: true,
            mode: libmdbx::Mode::ReadWrite {
                sync_mode: libmdbx::SyncMode::UtterlyNoSync,
            },
            ..Default::default()
        };

        env.set_flags(db_flags);

        // This is only required if multiple databases will be used in the environment.
        env.set_max_dbs(max_dbs as usize);
        if let Some(max_readers) = max_readers {
            env.set_max_readers(max_readers);
        }

        let env = env.open(Path::new(path)).unwrap();

        let info = env.info().unwrap();
        let cur_mapsize = info.map_size();
        info!("MDBX memory map size: {}", cur_mapsize);

        let mdbx = MdbxEnvironment {
            env: Arc::new(env),
            path: path.to_string(),
        };
        if mdbx.need_resize(0) {
            info!("MDBX memory needs to be resized.");
        }

        Ok(mdbx)
    }

    pub(super) fn open_database(&self, name: String, flags: DatabaseFlags) -> MdbxDatabase {
        // This is an implicit transaction, so take the lock first.
        let mut db_flags = libmdbx::DatabaseFlags::CREATE;

        // Translate flags.
        if flags.contains(DatabaseFlags::DUPLICATE_KEYS) {
            db_flags.insert(libmdbx::DatabaseFlags::DUP_SORT);

            if flags.contains(DatabaseFlags::DUP_FIXED_SIZE_VALUES) {
                db_flags.insert(libmdbx::DatabaseFlags::DUP_FIXED);
            }

            //MDBX doesnt like this flag
            //if flags.contains(DatabaseFlags::DUP_UINT_VALUES) {
            //    db_flags.insert(libmdbx::DatabaseFlags::INTEGER_DUP);
            //}
        }
        if flags.contains(DatabaseFlags::UINT_KEYS) {
            db_flags.insert(libmdbx::DatabaseFlags::INTEGER_KEY);
        }

        // Create the database
        let txn = self.env.begin_rw_txn().unwrap();
        txn.create_db(Some(&name), db_flags).unwrap();
        txn.commit().unwrap();

        MdbxDatabase {
            db: name,
            flags: db_flags,
        }
    }

    pub(super) fn drop_database(self) -> io::Result<()> {
        fs::remove_dir_all(self.path())
    }

    fn path(&self) -> String {
        self.path.clone()
    }

    pub fn need_resize(&self, threshold_size: usize) -> bool {
        let info = self.env.info().unwrap();
        let stat = self.env.stat().unwrap();

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
pub struct MdbxDatabase {
    db: String,
    flags: libmdbx::DatabaseFlags,
}

pub struct MdbxReadTransaction<'env> {
    txn: mdbx::Transaction<'env, RO, NoWriteMap>,
}

impl<'env> MdbxReadTransaction<'env> {
    pub(super) fn new(env: &'env MdbxEnvironment) -> Self {
        MdbxReadTransaction {
            txn: env.env.begin_ro_txn().unwrap(),
        }
    }

    pub(super) fn get<K, V>(&self, db: &MdbxDatabase, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        let db = self.txn.open_db(Some(&db.db)).unwrap();

        let result: Option<Cow<[u8]>> = self
            .txn
            .get(&db, &AsDatabaseBytes::as_database_bytes(key))
            .unwrap();

        Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
    }

    pub(super) fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> MdbxCursor<'txn> {
        let db = self
            .txn
            .open_db(Some(&db.persistent().unwrap().db))
            .unwrap();

        let cursor = self.txn.cursor(&db).unwrap();
        MdbxCursor {
            raw: RawMDBXCursor::RoCursor(cursor),
        }
    }
}

impl<'env> fmt::Debug for MdbxReadTransaction<'env> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MdbxReadTransaction {{ txn: {:?} }}", self.txn)
    }
}

pub struct MdbxWriteTransaction<'env> {
    txn: mdbx::Transaction<'env, RW, NoWriteMap>,
}

impl<'env> MdbxWriteTransaction<'env> {
    pub(super) fn new(env: &'env MdbxEnvironment) -> Self {
        // Check for enough space before every write transaction.
        if env.need_resize(0) {
            log::error!("DB needs resize, resize not supported");
        }
        MdbxWriteTransaction {
            txn: env.env.begin_rw_txn().unwrap(),
        }
    }

    pub(super) fn get<K, V>(&self, db: &MdbxDatabase, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        let db = self.txn.create_db(Some(&db.db), db.flags).unwrap();

        let result: Option<Cow<[u8]>> = self
            .txn
            .get(&db, AsDatabaseBytes::as_database_bytes(key).as_ref())
            .unwrap();

        Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
    }

    pub(super) fn put_reserve<K, V>(&mut self, db: &MdbxDatabase, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: IntoDatabaseValue + ?Sized,
    {
        let db = self.txn.create_db(Some(&db.db), db.flags).unwrap();

        let key = AsDatabaseBytes::as_database_bytes(key);
        let value_size = IntoDatabaseValue::database_byte_size(value);

        let bytes: &mut [u8] = self
            .txn
            .reserve(&db, key, value_size, WriteFlags::empty())
            .unwrap();

        IntoDatabaseValue::copy_into_database(value, bytes);
    }

    pub(super) fn put<K, V>(&mut self, db: &MdbxDatabase, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let db = self.txn.create_db(Some(&db.db), db.flags).unwrap();

        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);

        self.txn.put(&db, key, value, WriteFlags::empty()).unwrap();
    }

    pub(super) fn remove<K>(&mut self, db: &MdbxDatabase, key: &K)
    where
        K: AsDatabaseBytes + ?Sized,
    {
        let db = self.txn.create_db(Some(&db.db), db.flags).unwrap();

        self.txn
            .del(&db, AsDatabaseBytes::as_database_bytes(key).as_ref(), None)
            .unwrap();
    }

    pub(super) fn remove_item<K, V>(&mut self, db: &MdbxDatabase, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let db = self.txn.create_db(Some(&db.db), db.flags).unwrap();

        self.txn
            .del(
                &db,
                AsDatabaseBytes::as_database_bytes(key).as_ref(),
                Some(AsDatabaseBytes::as_database_bytes(value).as_ref()),
            )
            .unwrap();
    }

    pub(super) fn commit(self) {
        self.txn.commit().unwrap();
    }

    pub(super) fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> MdbxCursor<'txn> {
        let db = self
            .txn
            .open_db(Some(&db.persistent().unwrap().db))
            .unwrap();

        let cursor = self.txn.cursor(&db).unwrap();

        MdbxCursor {
            raw: RawMDBXCursor::RwCursor(cursor),
        }
    }

    pub(super) fn write_cursor<'txn, 'db>(&'txn self, db: &'db Database) -> MdbxWriteCursor<'txn> {
        let db = self
            .txn
            .open_db(Some(&db.persistent().unwrap().db))
            .unwrap();
        let cursor = self.txn.cursor(&db).unwrap();

        MdbxWriteCursor {
            raw: RawWriteLmdbCursor { cursor },
        }
    }
}

impl<'env> fmt::Debug for MdbxWriteTransaction<'env> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "LmdbWriteTransaction {{ txn: {:?} }}", self.txn)
    }
}

pub enum RawMDBXCursor<'txn> {
    RoCursor(libmdbx::Cursor<'txn, RO>),
    RwCursor(libmdbx::Cursor<'txn, RW>),
}

pub struct RawWriteLmdbCursor<'txn> {
    cursor: libmdbx::Cursor<'txn, RW>,
}

impl<'txn, 'db> RawReadCursor for RawMDBXCursor<'txn> {
    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<DbKvPair> = rocursor.first().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<DbKvPair> = rwcursor.first().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<Cow<[u8]>> = rocursor.first_dup().unwrap();
                Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<Cow<[u8]>> = rwcursor.first_dup().unwrap();
                Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
            }
        }
    }

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<DbKvPair> = rocursor.last().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<DbKvPair> = rwcursor.last().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<Cow<[u8]>> = rocursor.last_dup().unwrap();
                Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<Cow<[u8]>> = rwcursor.last_dup().unwrap();
                Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
            }
        }
    }

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<DbKvPair> = rocursor.get_current().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<DbKvPair> = rwcursor.get_current().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<DbKvPair> = rocursor.next().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<DbKvPair> = rwcursor.next().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<DbKvPair> = rocursor.next_dup().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<DbKvPair> = rwcursor.next_dup().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<DbKvPair> = rocursor.next_nodup().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<DbKvPair> = rwcursor.next_nodup().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<DbKvPair> = rocursor.prev().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<DbKvPair> = rwcursor.prev().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<DbKvPair> = rocursor.prev_dup().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<DbKvPair> = rwcursor.prev_dup().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<DbKvPair> = rocursor.prev_nodup().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<DbKvPair> = rwcursor.prev_nodup().unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let key = AsDatabaseBytes::as_database_bytes(key);
                let result: Option<Cow<[u8]>> = rocursor.set(key.as_ref()).unwrap();
                Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
            }
            Self::RwCursor(rwcursor) => {
                let key = AsDatabaseBytes::as_database_bytes(key);
                let result: Option<Cow<[u8]>> = rwcursor.set(key.as_ref()).unwrap();
                Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
            }
        }
    }

    fn seek_key_both<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let key = AsDatabaseBytes::as_database_bytes(key);
                let result: Option<DbKvPair> = rocursor.set_key(key.as_ref()).unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let key = AsDatabaseBytes::as_database_bytes(key);
                let result: Option<DbKvPair> = rwcursor.set_key(key.as_ref()).unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        match self {
            Self::RoCursor(rocursor) => {
                let key = AsDatabaseBytes::as_database_bytes(key);
                let result: Option<DbKvPair> = rocursor.set_range(key.as_ref()).unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
            Self::RwCursor(rwcursor) => {
                let key = AsDatabaseBytes::as_database_bytes(key);
                let result: Option<DbKvPair> = rwcursor.set_range(key.as_ref()).unwrap();
                let (key, value) = result?;
                Some((
                    FromDatabaseValue::copy_from_database(&key).unwrap(),
                    FromDatabaseValue::copy_from_database(&value).unwrap(),
                ))
            }
        }
    }

    fn count_duplicates(&mut self) -> usize {
        match self {
            Self::RoCursor(rocursor) => {
                let result: Option<DbKvPair> = rocursor.get_current().unwrap();

                if let Some((key, _)) = result {
                    return rocursor.iter_dup_of::<(), ()>(&key).count();
                } else {
                    0_usize
                }
            }
            Self::RwCursor(rwcursor) => {
                let result: Option<DbKvPair> = rwcursor.get_current().unwrap();

                if let Some((key, _)) = result {
                    return rwcursor.iter_dup_of::<(), ()>(&key).count();
                } else {
                    0_usize
                }
            }
        }
    }
}

impl<'txn, 'db> RawReadCursor for RawWriteLmdbCursor<'txn> {
    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.first().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        let result: Option<Cow<[u8]>> = self.cursor.first_dup().unwrap();
        Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
    }

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.last().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        let result: Option<Cow<[u8]>> = self.cursor.last_dup().unwrap();
        Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
    }

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.get_current().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.next().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.next_dup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.next_nodup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.prev().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.prev_dup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<DbKvPair> = self.cursor.prev_nodup().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let result: Option<Cow<[u8]>> = self.cursor.set(key.as_ref()).unwrap();
        Some(FromDatabaseValue::copy_from_database(&result?).unwrap())
    }

    fn seek_key_both<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let result: Option<DbKvPair> = self.cursor.set_key(key.as_ref()).unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let result: Option<DbKvPair> = self.cursor.set_range(key.as_ref()).unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(&key).unwrap(),
            FromDatabaseValue::copy_from_database(&value).unwrap(),
        ))
    }

    fn count_duplicates(&mut self) -> usize {
        0
    }
}

pub struct MdbxCursor<'txn> {
    raw: RawMDBXCursor<'txn>,
}

impl_read_cursor_from_raw!(MdbxCursor<'txn>, raw);

pub struct MdbxWriteCursor<'txn> {
    raw: RawWriteLmdbCursor<'txn>,
}

impl_read_cursor_from_raw!(MdbxWriteCursor<'txn>, raw);

impl<'txn, 'db> WriteCursorTrait for MdbxWriteCursor<'txn> {
    fn remove(&mut self) {
        self.raw.cursor.del(WriteFlags::empty()).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_can_save_basic_objects() {
        let env = MdbxEnvironment::new("./test", 0, 1).unwrap();
        {
            let db = env.open_database("test".to_string());

            // Read non-existent value.
            {
                let tx = ReadTransaction::new(&env);
                assert!(tx.get::<str, String>(&db, "test").is_none());
            }

            // Read non-existent value.
            let mut tx = WriteTransaction::new(&env);
            assert!(tx.get::<str, String>(&db, "test").is_none());

            // Write and read value.
            tx.put_reserve(&db, "test", "one");
            assert_eq!(tx.get::<str, String>(&db, "test"), Some("one".to_string()));
            // Overwrite and read value.
            tx.put_reserve(&db, "test", "two");
            assert_eq!(tx.get::<str, String>(&db, "test"), Some("two".to_string()));
            tx.commit();

            // Read value.
            let tx = ReadTransaction::new(&env);
            assert_eq!(tx.get::<str, String>(&db, "test"), Some("two".to_string()));
            tx.close();

            // Remove value.
            let mut tx = WriteTransaction::new(&env);
            tx.remove(&db, "test");
            assert!(tx.get::<str, String>(&db, "test").is_none());
            tx.commit();

            // Check removal.
            {
                let tx = ReadTransaction::new(&env);
                assert!(tx.get::<str, String>(&db, "test").is_none());
            }

            // Write and abort.
            let mut tx = WriteTransaction::new(&env);
            tx.put_reserve(&db, "test", "one");
            tx.abort();

            // Check aborted transaction.
            let tx = ReadTransaction::new(&env);
            assert!(tx.get::<str, String>(&db, "test").is_none());
        }

        env.drop_database().unwrap();
    }

    #[test]
    fn isolation_test() {
        let env = MdbxEnvironment::new("./test2", 0, 1).unwrap();
        {
            let db = env.open_database("test".to_string());

            // Read non-existent value.
            let tx = ReadTransaction::new(&env);
            assert!(tx.get::<str, String>(&db, "test").is_none());

            // WriteTransaction.
            let mut txw = WriteTransaction::new(&env);
            assert!(txw.get::<str, String>(&db, "test").is_none());
            txw.put_reserve(&db, "test", "one");
            assert_eq!(txw.get::<str, String>(&db, "test"), Some("one".to_string()));

            // ReadTransaction should still have the old state.
            assert!(tx.get::<str, String>(&db, "test").is_none());

            // Commit WriteTransaction.
            txw.commit();

            // ReadTransaction should still have the old state.
            assert!(tx.get::<str, String>(&db, "test").is_none());

            // Have a new ReadTransaction read the new state.
            let tx2 = ReadTransaction::new(&env);
            assert_eq!(tx2.get::<str, String>(&db, "test"), Some("one".to_string()));
        }

        env.drop_database().unwrap();
    }

    #[test]
    fn duplicates_test() {
        let env = MdbxEnvironment::new("./test3", 0, 1).unwrap();
        {
            let db = env.open_database_with_flags(
                "test".to_string(),
                DatabaseFlags::DUPLICATE_KEYS | DatabaseFlags::DUP_UINT_VALUES,
            );

            // Write one value.
            let mut txw = WriteTransaction::new(&env);
            assert!(txw.get::<str, u32>(&db, "test").is_none());
            txw.put::<str, u32>(&db, "test", &125);
            assert_eq!(txw.get::<str, u32>(&db, "test"), Some(125));
            txw.commit();

            // Have a new ReadTransaction read the new state.
            {
                let tx = ReadTransaction::new(&env);
                assert_eq!(tx.get::<str, u32>(&db, "test"), Some(125));
            }

            // Write a second smaller value.
            let mut txw = WriteTransaction::new(&env);
            assert_eq!(txw.get::<str, u32>(&db, "test"), Some(125));
            txw.put::<str, u32>(&db, "test", &12);
            assert_eq!(txw.get::<str, u32>(&db, "test"), Some(12));
            txw.commit();

            // Have a new ReadTransaction read the smaller value.
            {
                let tx = ReadTransaction::new(&env);
                assert_eq!(tx.get::<str, u32>(&db, "test"), Some(12));
            }

            // Remove smaller value and write larger value.
            let mut txw = WriteTransaction::new(&env);
            assert_eq!(txw.get::<str, u32>(&db, "test"), Some(12));
            txw.remove_item::<str, u32>(&db, "test", &12);
            txw.put::<str, u32>(&db, "test", &5783);
            assert_eq!(txw.get::<str, u32>(&db, "test"), Some(125));
            txw.commit();

            // Have a new ReadTransaction read the smallest value.
            {
                let tx = ReadTransaction::new(&env);
                assert_eq!(tx.get::<str, u32>(&db, "test"), Some(125));
            }

            // Remove everything.
            let mut txw = WriteTransaction::new(&env);
            assert_eq!(txw.get::<str, u32>(&db, "test"), Some(125));
            txw.remove::<str>(&db, "test");
            assert!(txw.get::<str, u32>(&db, "test").is_none());
            txw.commit();

            // Have a new ReadTransaction read the new state.
            {
                let tx = ReadTransaction::new(&env);
                assert!(tx.get::<str, u32>(&db, "test").is_none());
            }
        }

        env.drop_database().unwrap();
    }

    #[test]
    fn cursor_test() {
        let env = MdbxEnvironment::new("./test4", 0, 1).unwrap();
        {
            let db = env.open_database_with_flags(
                "test".to_string(),
                DatabaseFlags::DUPLICATE_KEYS | DatabaseFlags::DUP_UINT_VALUES,
            );

            let test1: String = "test1".to_string();
            let test2: String = "test2".to_string();

            // Write some values.
            let mut txw = WriteTransaction::new(&env);
            assert!(txw.get::<str, u32>(&db, "test").is_none());
            txw.put::<str, u32>(&db, "test1", &125);
            txw.put::<str, u32>(&db, "test1", &12);
            txw.put::<str, u32>(&db, "test1", &5783);
            txw.put::<str, u32>(&db, "test2", &5783);
            txw.commit();

            // Have a new ReadTransaction read the new state.
            let tx = ReadTransaction::new(&env);
            let mut cursor = tx.cursor(&db);
            assert_eq!(cursor.first::<String, u32>(), Some((test1.clone(), 12)));
            assert_eq!(cursor.last::<String, u32>(), Some((test2.clone(), 5783)));
            assert_eq!(cursor.prev::<String, u32>(), Some((test1.clone(), 5783)));
            assert_eq!(cursor.first_duplicate::<u32>(), Some(12));
            assert_eq!(
                cursor.next_duplicate::<String, u32>(),
                Some((test1.clone(), 125))
            );
            assert_eq!(
                cursor.prev_duplicate::<String, u32>(),
                Some((test1.clone(), 12))
            );
            assert_eq!(
                cursor.next_no_duplicate::<String, u32>(),
                Some((test2.clone(), 5783))
            );
            assert!(cursor.seek_key::<str, u32>("test").is_none());
            assert_eq!(cursor.seek_key::<str, u32>("test1"), Some(12));
            assert_eq!(cursor.count_duplicates(), 3);
            assert_eq!(cursor.last_duplicate::<u32>(), Some(5783));
            //            assert_eq!(cursor.seek_key_both::<String, u32>(&test1), Some((test1.clone(), 12)));

            assert_eq!(
                cursor.get_current::<String, u32>(),
                //Some((test1.clone(), 125))
                Some((test1.clone(), 5783))
            );

            assert_eq!(cursor.get_current::<String, u32>(), Some((test1, 5783)));
            assert!(cursor.prev_no_duplicate::<String, u32>().is_none());
            assert_eq!(cursor.next::<String, u32>(), Some((test2, 5783)));
            //            assert_eq!(cursor.seek_range_key::<String, u32>("test"), Some((test1.clone(), 12)));
        }

        env.drop_database().unwrap();
    }
}
