use std::sync::Arc;

use tempfile::TempDir;

use super::mdbx::*;
use super::*;
use crate::cursor::{ReadCursor, WriteCursor as WriteCursorTrait};

#[derive(Debug)]
pub struct VolatileEnvironment {
    temp_dir: Arc<TempDir>,
    env: MdbxEnvironment,
}

impl Clone for VolatileEnvironment {
    fn clone(&self) -> Self {
        Self {
            temp_dir: Arc::clone(&self.temp_dir),
            env: self.env.clone(),
        }
    }
}

impl VolatileEnvironment {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(max_dbs: u32) -> Result<Environment, Error> {
        let temp_dir = TempDir::new().map_err(Error::CreateDirectory)?;
        let env = MdbxEnvironment::new_mdbx_environment(temp_dir.path(), 0, max_dbs, None)?;
        Ok(Environment::Volatile(VolatileEnvironment {
            temp_dir: Arc::new(temp_dir),
            env,
        }))
    }

    pub fn with_max_readers(max_dbs: u32, max_readers: u32) -> Result<Environment, Error> {
        let temp_dir = TempDir::new().map_err(Error::CreateDirectory)?;
        let env =
            MdbxEnvironment::new_mdbx_environment(temp_dir.path(), 0, max_dbs, Some(max_readers))?;
        Ok(Environment::Volatile(VolatileEnvironment {
            temp_dir: Arc::new(temp_dir),
            env,
        }))
    }

    pub(super) fn open_database(&self, name: String, flags: DatabaseFlags) -> VolatileDatabase {
        VolatileDatabase(self.env.open_database(name, flags))
    }
}

#[derive(Debug)]
pub struct VolatileDatabase(MdbxDatabase);

impl VolatileDatabase {
    pub(super) fn as_mdbx(&self) -> &MdbxDatabase {
        &self.0
    }
}

#[derive(Debug)]
pub struct VolatileReadTransaction<'env>(MdbxReadTransaction<'env>);

impl<'env> VolatileReadTransaction<'env> {
    pub(super) fn new(env: &'env VolatileEnvironment) -> Self {
        VolatileReadTransaction(MdbxReadTransaction::new(&env.env))
    }

    pub(super) fn get<K, V>(&self, db: &VolatileDatabase, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        self.0.get(&db.0, key)
    }

    pub(super) fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> VolatileCursor<'txn> {
        VolatileCursor(self.0.cursor(db))
    }
}

#[derive(Debug)]
pub struct VolatileWriteTransaction<'env>(MdbxWriteTransaction<'env>);

impl<'env> VolatileWriteTransaction<'env> {
    #[allow(clippy::new_ret_no_self)]
    pub(super) fn new(env: &'env VolatileEnvironment) -> Self {
        VolatileWriteTransaction(MdbxWriteTransaction::new(&env.env))
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

    pub(super) fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> VolatileCursor<'txn> {
        VolatileCursor(self.0.cursor(db))
    }

    pub(super) fn write_cursor<'txn, 'db>(
        &'txn self,
        db: &'db Database,
    ) -> VolatileWriteCursor<'txn> {
        VolatileWriteCursor(self.0.write_cursor(db))
    }
}

pub struct VolatileCursor<'txn>(MdbxCursor<'txn>);

impl<'txn> ReadCursor for VolatileCursor<'txn> {
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

pub struct VolatileWriteCursor<'txn>(MdbxWriteCursor<'txn>);

impl<'txn, 'db> ReadCursor for VolatileWriteCursor<'txn> {
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

impl<'txn> WriteCursorTrait for VolatileWriteCursor<'txn> {
    fn remove(&mut self) {
        self.0.remove()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nimiq_test_log::test;

    #[test]
    fn it_can_save_basic_objects() {
        let env = VolatileEnvironment::new(1).unwrap();
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
    }

    #[test]
    fn isolation_test() {
        let env = VolatileEnvironment::with_max_readers(1, 126).unwrap();
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
    }

    #[test]
    fn duplicates_test() {
        let env = VolatileEnvironment::with_max_readers(1, 126).unwrap();
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
    }

    #[test]
    fn cursor_test() {
        let env = VolatileEnvironment::with_max_readers(1, 126).unwrap();
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
            //assert_eq!(cursor.count_duplicates(), 3);
            assert_eq!(cursor.last_duplicate::<u32>(), Some(5783));
            //            assert_eq!(cursor.seek_key_both::<String, u32>(&test1), Some((test1.clone(), 12)));
            //assert!(!cursor.seek_key_value::<str, u32>("test1", &15));
            //assert!(cursor.seek_key_value::<str, u32>("test1", &125));
            assert_eq!(
                cursor.get_current::<String, u32>(),
                //Some((test1.clone(), 125))
                Some((test1.clone(), 5783))
            );
            //assert_eq!(
            //    cursor.seek_key_nearest_value::<str, u32>("test1", &126),
            //    Some(5783)
            //);
            assert_eq!(cursor.get_current::<String, u32>(), Some((test1, 5783)));
            assert!(cursor.prev_no_duplicate::<String, u32>().is_none());
            assert_eq!(cursor.next::<String, u32>(), Some((test2, 5783)));
            //            assert_eq!(cursor.seek_range_key::<String, u32>("test"), Some((test1.clone(), 12)));
        }
    }
}
