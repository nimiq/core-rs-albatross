use std::error::Error;
use std::fmt;
use std::io;
use std::sync::Arc;

use tempfile::TempDir;

use super::lmdb::*;
use super::*;
use crate::cursor::{ReadCursor, WriteCursor as WriteCursorTrait};

#[derive(Debug)]
pub struct VolatileEnvironment {
    temp_dir: Arc<TempDir>,
    env: LmdbEnvironment,
}

impl Clone for VolatileEnvironment {
    fn clone(&self) -> Self {
        Self {
            temp_dir: Arc::clone(&self.temp_dir),
            env: self.env.clone(),
        }
    }
}

#[derive(Debug)]
pub enum VolatileDatabaseError {
    IoError(io::Error),
    LmdbError(LmdbError),
}

impl fmt::Display for VolatileDatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            VolatileDatabaseError::IoError(e) => e.fmt(f),
            VolatileDatabaseError::LmdbError(e) => e.fmt(f),
        }
    }
}

impl Error for VolatileDatabaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            VolatileDatabaseError::IoError(e) => Some(e),
            VolatileDatabaseError::LmdbError(e) => Some(e),
        }
    }
}

impl VolatileEnvironment {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        max_dbs: u32,
        max_readers: Option<u32>,
    ) -> Result<Environment, VolatileDatabaseError> {
        let temp_dir = TempDir::new().map_err(VolatileDatabaseError::IoError)?;
        let path = temp_dir
            .path()
            .to_str()
            .ok_or_else(|| {
                VolatileDatabaseError::IoError(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Path cannot be converted into a string.",
                ))
            })?
            .to_string();
        Ok(Environment::Volatile(VolatileEnvironment {
            temp_dir: Arc::new(temp_dir),
            env: LmdbEnvironment::new_lmdb_environment(
                &path,
                0,
                max_dbs,
                max_readers,
                open::NOSYNC | open::WRITEMAP,
            )
            .map_err(VolatileDatabaseError::LmdbError)?,
        }))
    }

    pub fn new_with_lmdb_flags(
        max_dbs: u32,
        max_readers: Option<u32>,
        flags: open::Flags,
    ) -> Result<Environment, VolatileDatabaseError> {
        let temp_dir = TempDir::new().map_err(VolatileDatabaseError::IoError)?;
        let path = temp_dir
            .path()
            .to_str()
            .ok_or_else(|| {
                VolatileDatabaseError::IoError(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Path cannot be converted into a string.",
                ))
            })?
            .to_string();
        Ok(Environment::Volatile(VolatileEnvironment {
            temp_dir: Arc::new(temp_dir),
            env: LmdbEnvironment::new_lmdb_environment(
                &path,
                0,
                max_dbs,
                max_readers,
                flags | open::NOSYNC | open::WRITEMAP,
            )
            .map_err(VolatileDatabaseError::LmdbError)?,
        }))
    }

    pub(super) fn open_database(&self, name: String, flags: DatabaseFlags) -> VolatileDatabase {
        VolatileDatabase(self.env.open_database(name, flags))
    }

    pub(super) fn drop_database(self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct VolatileDatabase(LmdbDatabase);

impl VolatileDatabase {
    pub(super) fn as_lmdb<'a>(&'a self) -> &'a LmdbDatabase {
        &self.0
    }
}

#[derive(Debug)]
pub struct VolatileReadTransaction<'env>(LmdbReadTransaction<'env>);

impl<'env> VolatileReadTransaction<'env> {
    pub(super) fn new(env: &'env VolatileEnvironment) -> Self {
        VolatileReadTransaction(LmdbReadTransaction::new(&env.env))
    }

    pub(super) fn get<K, V>(&self, db: &VolatileDatabase, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        self.0.get(&db.0, key)
    }

    pub(super) fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> VolatileCursor<'txn, 'db> {
        VolatileCursor(self.0.cursor(db))
    }
}

#[derive(Debug)]
pub struct VolatileWriteTransaction<'env>(LmdbWriteTransaction<'env>);

impl<'env> VolatileWriteTransaction<'env> {
    #[allow(clippy::new_ret_no_self)]
    pub(super) fn new(env: &'env VolatileEnvironment) -> Self {
        VolatileWriteTransaction(LmdbWriteTransaction::new(&env.env))
    }

    pub(super) fn get<K, V>(&self, db: &VolatileDatabase, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        self.0.get(&db.0, key)
    }

    pub(super) fn put_reserve<K, V>(&mut self, db: &VolatileDatabase, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: IntoDatabaseValue + ?Sized,
    {
        self.0.put_reserve(&db.0, key, value)
    }

    pub(super) fn put<K, V>(&mut self, db: &VolatileDatabase, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        self.0.put(&db.0, key, value)
    }

    pub(super) fn remove<K>(&mut self, db: &VolatileDatabase, key: &K)
    where
        K: AsDatabaseBytes + ?Sized,
    {
        self.0.remove(&db.0, key)
    }

    pub(super) fn remove_item<K, V>(&mut self, db: &VolatileDatabase, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        self.0.remove_item(&db.0, key, value)
    }

    pub(super) fn commit(self) {
        self.0.commit()
    }

    pub(super) fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> VolatileCursor<'txn, 'db> {
        VolatileCursor(self.0.cursor(db))
    }

    pub(super) fn write_cursor<'txn, 'db>(
        &'txn self,
        db: &'db Database,
    ) -> VolatileWriteCursor<'txn, 'db> {
        VolatileWriteCursor(self.0.write_cursor(db))
    }
}

pub struct VolatileCursor<'txn, 'db>(LmdbCursor<'txn, 'db>);

impl<'txn, 'db> ReadCursor for VolatileCursor<'txn, 'db> {
    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.first()
    }

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        self.0.first_duplicate()
    }

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.last()
    }

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        self.0.last_duplicate()
    }

    fn seek_key_value<K, V>(&mut self, key: &K, value: &V) -> bool
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        self.0.seek_key_value(key, value)
    }

    fn seek_key_nearest_value<K, V>(&mut self, key: &K, value: &V) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + FromDatabaseValue,
    {
        self.0.seek_key_nearest_value(key, value)
    }

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.get_current()
    }

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.next()
    }

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.next_duplicate()
    }

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.next_no_duplicate()
    }

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.prev()
    }

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.prev_duplicate()
    }

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.prev_no_duplicate()
    }

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        self.0.seek_key(key)
    }

    fn seek_key_both<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.seek_key_both(key)
    }

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.seek_range_key(key)
    }

    fn count_duplicates(&mut self) -> usize {
        self.0.count_duplicates()
    }
}

pub struct VolatileWriteCursor<'txn, 'db>(LmdbWriteCursor<'txn, 'db>);

impl<'txn, 'db> ReadCursor for VolatileWriteCursor<'txn, 'db> {
    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.first()
    }

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        self.0.first_duplicate()
    }

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.last()
    }

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        self.0.last_duplicate()
    }

    fn seek_key_value<K, V>(&mut self, key: &K, value: &V) -> bool
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        self.0.seek_key_value(key, value)
    }

    fn seek_key_nearest_value<K, V>(&mut self, key: &K, value: &V) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + FromDatabaseValue,
    {
        self.0.seek_key_nearest_value(key, value)
    }

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.get_current()
    }

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.next()
    }

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.next_duplicate()
    }

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.next_no_duplicate()
    }

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.prev()
    }

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.prev_duplicate()
    }

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.prev_no_duplicate()
    }

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        self.0.seek_key(key)
    }

    fn seek_key_both<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.seek_key_both(key)
    }

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        self.0.seek_range_key(key)
    }

    fn count_duplicates(&mut self) -> usize {
        self.0.count_duplicates()
    }
}

impl<'txn, 'db> WriteCursorTrait for VolatileWriteCursor<'txn, 'db> {
    fn remove(&mut self) {
        self.0.remove()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_can_save_basic_objects() {
        let env = VolatileEnvironment::new(1, None).unwrap();
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
        let env = VolatileEnvironment::new_with_lmdb_flags(1, None, open::NOTLS).unwrap();
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
        let env = VolatileEnvironment::new_with_lmdb_flags(1, None, open::NOTLS).unwrap();
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
        let env = VolatileEnvironment::new_with_lmdb_flags(1, None, open::NOTLS).unwrap();
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
