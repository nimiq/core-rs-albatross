use std::cmp;
use std::fmt;
use std::fs;
use std::sync::Arc;

// re export the lmdb error
pub use lmdb_zero::open;
use lmdb_zero::traits::LmdbResultExt;
pub use lmdb_zero::Error as LmdbError;
use rand::{thread_rng, Rng};

use crate::cursor::{RawReadCursor, ReadCursor, WriteCursor as WriteCursorTrait};

use super::*;

#[derive(Debug)]
pub struct LmdbEnvironment {
    env: Arc<lmdb_zero::Environment>,
    creation_gate: Arc<parking_lot::RwLock<()>>,
}

impl Clone for LmdbEnvironment {
    fn clone(&self) -> Self {
        Self {
            env: Arc::clone(&self.env),
            creation_gate: Arc::clone(&self.creation_gate),
        }
    }
}

impl LmdbEnvironment {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        path: &str,
        size: usize,
        max_dbs: u32,
        flags: open::Flags,
    ) -> Result<Environment, LmdbError> {
        Ok(Environment::Persistent(
            LmdbEnvironment::new_lmdb_environment(path, size, max_dbs, flags)?,
        ))
    }

    pub(super) fn new_lmdb_environment(
        path: &str,
        size: usize,
        max_dbs: u32,
        flags: open::Flags,
    ) -> Result<Self, LmdbError> {
        fs::create_dir_all(path).unwrap();

        let mut env = lmdb_zero::EnvBuilder::new()?;
        env.set_maxdbs(max_dbs)?;
        let env = unsafe { env.open(path, flags, 0o600)? };

        let info = env.info()?;
        let cur_mapsize = info.mapsize;
        if cur_mapsize < size {
            unsafe { env.set_mapsize(size)? };
            let info = env.info()?;
            let cur_mapsize = info.mapsize;
            info!("LMDB memory map size: {}", cur_mapsize);
        }

        let lmdb = LmdbEnvironment {
            env: Arc::new(env),
            creation_gate: Arc::new(parking_lot::RwLock::new(())),
        };
        if lmdb.need_resize(0) {
            info!("LMDB memory needs to be resized.");
            lmdb.do_resize(0);
        }

        Ok(lmdb)
    }

    pub(super) fn open_database(&self, name: String, flags: DatabaseFlags) -> LmdbDatabase {
        // This is an implicit transaction, so take the lock first.
        let _guard = self.creation_gate.read();
        let mut db_flags = lmdb_zero::db::CREATE;

        // Translate flags.
        if flags.contains(DatabaseFlags::DUPLICATE_KEYS) {
            db_flags.insert(lmdb_zero::db::DUPSORT);

            if flags.contains(DatabaseFlags::DUP_FIXED_SIZE_VALUES) {
                db_flags.insert(lmdb_zero::db::DUPFIXED);
            }

            if flags.contains(DatabaseFlags::DUP_UINT_VALUES) {
                db_flags.insert(lmdb_zero::db::INTEGERDUP);
            }
        }
        if flags.contains(DatabaseFlags::UINT_KEYS) {
            db_flags.insert(lmdb_zero::db::INTEGERKEY);
        }

        LmdbDatabase {
            db: lmdb_zero::Database::open(
                Arc::clone(&self.env),
                Some(&name),
                &lmdb_zero::DatabaseOptions::new(db_flags),
            )
            .unwrap(),
        }
    }

    pub(super) fn drop_database(self) -> io::Result<()> {
        fs::remove_dir_all(self.path().as_ref())
    }

    fn path(&self) -> Cow<str> {
        self.env.path().unwrap().to_string_lossy()
    }

    pub fn do_resize(&self, increase_size: usize) {
        // Lock creation of new transactions until resize is finished.
        let _guard = self.creation_gate.write();
        let add_size: usize = cmp::max(1 << 30, increase_size);

        let available_space = fs2::available_space(self.path().as_ref());
        match available_space {
            Ok(available_space) => {
                let available_space = available_space as usize;
                // Check disk capacity.
                if available_space < add_size {
                    error!(
                        "Insufficient free space to extend database: {} MB available, {} MB needed.",
                        available_space >> 20,
                        add_size >> 20
                    );
                    return;
                }
            }
            Err(e) => {
                warn!("Unable to query free disk space:\n{:?}", e);
            }
        }

        let info = self.env.info().unwrap();
        let stat = self.env.stat().unwrap();

        let mut new_mapsize = info.mapsize + add_size;
        new_mapsize += new_mapsize % (stat.psize as usize);

        // TODO: Should we handle the error?
        unsafe {
            self.env.set_mapsize(new_mapsize).unwrap();
        }

        info!(
            "LMDB Mapsize increased. Old: {} MiB, New: {} MiB",
            info.mapsize / (1024 * 1024),
            new_mapsize / (1024 * 1024)
        );
    }

    pub fn need_resize(&self, threshold_size: usize) -> bool {
        let info = self.env.info().unwrap();
        let stat = self.env.stat().unwrap();

        let size_used = (stat.psize as usize) * (info.last_pgno + 1);

        if threshold_size > 0 && info.mapsize - size_used < threshold_size {
            info!("DB resize (threshold-based)");
            info!("DB map size: {}", info.mapsize);
            info!("Space used: {}", size_used);
            info!("Space remaining: {}", info.mapsize - size_used);
            info!("Size threshold: {}", threshold_size);
            return true;
        }

        let mut rng = thread_rng();
        let resize_percent: f64 = rng.gen_range(0.6, 0.9);

        if (size_used as f64) / (info.mapsize as f64) > resize_percent {
            info!("DB resize (percent-based)");
            info!("DB map size: {}", info.mapsize);
            info!("Space used: {}", size_used);
            info!("Space remaining: {}", info.mapsize - size_used);
            info!(
                "Percent used: {:.2}",
                (size_used as f64) / (info.mapsize as f64)
            );
            return true;
        }

        false
    }
}

#[derive(Debug)]
pub struct LmdbDatabase {
    db: lmdb_zero::Database<'static>,
}

pub struct LmdbReadTransaction<'env> {
    txn: lmdb_zero::ReadTransaction<'env>,
    #[allow(dead_code)]
    guard: parking_lot::RwLockReadGuard<'env, ()>,
}

impl<'env> LmdbReadTransaction<'env> {
    pub(super) fn new(env: &'env LmdbEnvironment) -> Self {
        // This is an implicit transaction, so take the lock first.
        let guard = env.creation_gate.read();
        LmdbReadTransaction {
            txn: lmdb_zero::ReadTransaction::new(Arc::clone(&env.env)).unwrap(),
            guard,
        }
    }

    pub(super) fn get<K, V>(&self, db: &LmdbDatabase, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        let access = self.txn.access();
        let result: Option<&[u8]> = access
            .get(&db.db, AsDatabaseBytes::as_database_bytes(key).as_ref())
            .to_opt()
            .unwrap();
        Some(FromDatabaseValue::copy_from_database(result?).unwrap())
    }

    pub(super) fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> LmdbCursor<'txn, 'db> {
        let cursor = self.txn.cursor(&db.persistent().unwrap().db).unwrap();
        LmdbCursor {
            raw: RawLmdbCursor { cursor },
            txn: &self.txn,
        }
    }
}

impl<'env> fmt::Debug for LmdbReadTransaction<'env> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "LmdbReadTransaction {{ txn: {:?} }}", self.txn)
    }
}

pub struct LmdbWriteTransaction<'env> {
    txn: lmdb_zero::WriteTransaction<'env>,
    #[allow(dead_code)]
    guard: parking_lot::RwLockReadGuard<'env, ()>,
}

impl<'env> LmdbWriteTransaction<'env> {
    pub(super) fn new(env: &'env LmdbEnvironment) -> Self {
        // Check for enough space before every write transaction.
        if env.need_resize(0) {
            env.do_resize(0);
        }
        let guard = env.creation_gate.read();
        LmdbWriteTransaction {
            txn: lmdb_zero::WriteTransaction::new(Arc::clone(&env.env)).unwrap(),
            guard,
        }
    }

    pub(super) fn get<K, V>(&self, db: &LmdbDatabase, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        let access = self.txn.access();
        let result: Option<&[u8]> = access
            .get(&db.db, AsDatabaseBytes::as_database_bytes(key).as_ref())
            .to_opt()
            .unwrap();
        Some(FromDatabaseValue::copy_from_database(result?).unwrap())
    }

    pub(super) fn put_reserve<K, V>(&mut self, db: &LmdbDatabase, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: IntoDatabaseValue + ?Sized,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let value_size = IntoDatabaseValue::database_byte_size(value);
        unsafe {
            let mut access = self.txn.access();
            let mut bytes: &mut [u8] = access
                .put_reserve_unsized(
                    &db.db,
                    key.as_ref(),
                    value_size,
                    lmdb_zero::put::Flags::empty(),
                )
                .unwrap();
            IntoDatabaseValue::copy_into_database(value, &mut bytes);
        }
    }

    pub(super) fn put<K, V>(&mut self, db: &LmdbDatabase, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);
        let mut access = self.txn.access();
        access
            .put(
                &db.db,
                key.as_ref(),
                value.as_ref(),
                lmdb_zero::put::Flags::empty(),
            )
            .unwrap();
    }

    pub(super) fn remove<K>(&mut self, db: &LmdbDatabase, key: &K)
    where
        K: AsDatabaseBytes + ?Sized,
    {
        let mut access = self.txn.access();
        access
            .del_key(&db.db, AsDatabaseBytes::as_database_bytes(key).as_ref())
            .to_opt()
            .unwrap();
    }

    pub(super) fn remove_item<K, V>(&mut self, db: &LmdbDatabase, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let mut access = self.txn.access();
        access
            .del_item(
                &db.db,
                AsDatabaseBytes::as_database_bytes(key).as_ref(),
                AsDatabaseBytes::as_database_bytes(value).as_ref(),
            )
            .to_opt()
            .unwrap();
    }

    pub(super) fn commit(self) {
        self.txn.commit().unwrap();
    }

    pub(super) fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> LmdbCursor<'txn, 'db> {
        let cursor = self.txn.cursor(&db.persistent().unwrap().db).unwrap();
        LmdbCursor {
            raw: RawLmdbCursor { cursor },
            txn: &self.txn,
        }
    }

    pub(super) fn write_cursor<'txn, 'db>(
        &'txn self,
        db: &'db Database,
    ) -> LmdbWriteCursor<'txn, 'db> {
        let cursor = self.txn.cursor(&db.persistent().unwrap().db).unwrap();
        LmdbWriteCursor {
            raw: RawLmdbCursor { cursor },
            txn: &self.txn,
        }
    }
}

impl<'env> fmt::Debug for LmdbWriteTransaction<'env> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "LmdbWriteTransaction {{ txn: {:?} }}", self.txn)
    }
}

pub struct RawLmdbCursor<'txn, 'db> {
    cursor: lmdb_zero::Cursor<'txn, 'db>,
}

impl<'txn, 'db> RawReadCursor for RawLmdbCursor<'txn, 'db> {
    fn first<K, V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<(&[u8], &[u8])> = self.cursor.first(access).to_opt().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn first_duplicate<V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        let result: Option<&[u8]> = self.cursor.first_dup(access).to_opt().unwrap();
        Some(FromDatabaseValue::copy_from_database(result?).unwrap())
    }

    fn last<K, V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<(&[u8], &[u8])> = self.cursor.last(access).to_opt().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn last_duplicate<V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        let result: Option<&[u8]> = self.cursor.last_dup(access).to_opt().unwrap();
        Some(FromDatabaseValue::copy_from_database(result?).unwrap())
    }

    fn seek_key_value<K, V>(&mut self, key: &K, value: &V) -> bool
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);
        let result = self.cursor.seek_kv(key.as_ref(), value.as_ref());
        result.is_ok()
    }

    fn seek_key_nearest_value<K, V>(
        &mut self,
        access: &lmdb_zero::ConstAccessor,
        key: &K,
        value: &V,
    ) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let value = AsDatabaseBytes::as_database_bytes(value);
        let result: Option<&[u8]> = self
            .cursor
            .seek_k_nearest_v(access, key.as_ref(), value.as_ref())
            .to_opt()
            .unwrap();
        Some(FromDatabaseValue::copy_from_database(result?).unwrap())
    }

    fn get_current<K, V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<(&[u8], &[u8])> = self.cursor.get_current(access).to_opt().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn next<K, V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<(&[u8], &[u8])> = self.cursor.next(access).to_opt().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn next_duplicate<K, V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<(&[u8], &[u8])> = self.cursor.next_dup(access).to_opt().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn next_no_duplicate<K, V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<(&[u8], &[u8])> = self.cursor.next_nodup(access).to_opt().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn prev<K, V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<(&[u8], &[u8])> = self.cursor.prev(access).to_opt().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn prev_duplicate<K, V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<(&[u8], &[u8])> = self.cursor.prev_dup(access).to_opt().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn prev_no_duplicate<K, V>(&mut self, access: &lmdb_zero::ConstAccessor) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let result: Option<(&[u8], &[u8])> = self.cursor.prev_nodup(access).to_opt().unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn seek_key<K, V>(&mut self, access: &lmdb_zero::ConstAccessor, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let result: Option<&[u8]> = self.cursor.seek_k(access, key.as_ref()).to_opt().unwrap();
        Some(FromDatabaseValue::copy_from_database(result?).unwrap())
    }

    fn seek_key_both<K, V>(&mut self, access: &lmdb_zero::ConstAccessor, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let result: Option<(&[u8], &[u8])> = self
            .cursor
            .seek_k_both(access, key.as_ref())
            .to_opt()
            .unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn seek_range_key<K, V>(&mut self, access: &lmdb_zero::ConstAccessor, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        let key = AsDatabaseBytes::as_database_bytes(key);
        let result: Option<(&[u8], &[u8])> = self
            .cursor
            .seek_range_k(access, key.as_ref())
            .to_opt()
            .unwrap();
        let (key, value) = result?;
        Some((
            FromDatabaseValue::copy_from_database(key).unwrap(),
            FromDatabaseValue::copy_from_database(value).unwrap(),
        ))
    }

    fn count_duplicates(&mut self) -> usize {
        self.cursor.count().unwrap()
    }
}

pub struct LmdbCursor<'txn, 'db> {
    raw: RawLmdbCursor<'txn, 'db>,
    txn: &'txn lmdb_zero::ConstTransaction<'txn>,
}

impl_read_cursor_from_raw!(LmdbCursor<'txn, 'db>, raw, txn);

pub struct LmdbWriteCursor<'txn, 'db> {
    raw: RawLmdbCursor<'txn, 'db>,
    txn: &'txn lmdb_zero::WriteTransaction<'txn>,
}

impl_read_cursor_from_raw!(LmdbWriteCursor<'txn, 'db>, raw, txn);

impl<'txn, 'db> WriteCursorTrait for LmdbWriteCursor<'txn, 'db> {
    fn remove(&mut self) {
        let mut access = self.txn.access();
        self.raw
            .cursor
            .del(&mut access, lmdb_zero::del::Flags::empty())
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_can_save_basic_objects() {
        let env = LmdbEnvironment::new("./test", 0, 1, open::Flags::empty()).unwrap();
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
        let env = LmdbEnvironment::new("./test2", 0, 1, open::NOTLS).unwrap();
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
        let env = LmdbEnvironment::new("./test3", 0, 1, open::NOTLS).unwrap();
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
        let env = LmdbEnvironment::new("./test4", 0, 1, open::NOTLS).unwrap();
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
            assert!(!cursor.seek_key_value::<str, u32>("test1", &15));
            assert!(cursor.seek_key_value::<str, u32>("test1", &125));
            assert_eq!(
                cursor.get_current::<String, u32>(),
                Some((test1.clone(), 125))
            );
            assert_eq!(
                cursor.seek_key_nearest_value::<str, u32>("test1", &126),
                Some(5783)
            );
            assert_eq!(cursor.get_current::<String, u32>(), Some((test1, 5783)));
            assert!(cursor.prev_no_duplicate::<String, u32>().is_none());
            assert_eq!(cursor.next::<String, u32>(), Some((test2, 5783)));
            //            assert_eq!(cursor.seek_range_key::<String, u32>("test"), Some((test1.clone(), 12)));
        }

        env.drop_database().unwrap();
    }
}
