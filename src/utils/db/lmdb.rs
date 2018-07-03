use super::*;
use lmdb_zero::traits::LmdbResultExt;
use std::fs;

#[derive(Debug)]
pub struct LmdbEnvironment {
    env: lmdb_zero::Environment,
}

impl LmdbEnvironment {
    pub fn new(path: &str, size: usize, max_dbs: u32) -> Environment {
        fs::create_dir_all(path).unwrap();

        let mut env = lmdb_zero::EnvBuilder::new().unwrap();
        env.set_mapsize(size).unwrap();
        env.set_maxdbs(max_dbs).unwrap();
        let env = unsafe {
            env.open(path, lmdb_zero::open::Flags::empty(), 0o600).unwrap()
        };
        return Environment::Persistent(LmdbEnvironment { env });
    }

    pub(in super) fn open_database<'env>(&'env self, name: String) -> LmdbDatabase<'env> {
        return LmdbDatabase { db: lmdb_zero::Database::open(&self.env, Some(&name), &lmdb_zero::DatabaseOptions::new(lmdb_zero::db::CREATE)).unwrap() };
    }

    pub(in super) fn drop_database(self) -> io::Result<()> {
        return fs::remove_dir_all(self.env.path().unwrap().to_string_lossy().as_ref());
    }
}

#[derive(Debug)]
pub struct LmdbDatabase<'env> {
    db: lmdb_zero::Database<'env>,
}

#[derive(Debug)]
pub struct LmdbReadTransaction<'env> {
    txn: lmdb_zero::ReadTransaction<'env>,
}

impl<'env> LmdbReadTransaction<'env> {
    pub(in super) fn new(env: &'env LmdbEnvironment) -> Self {
        return LmdbReadTransaction { txn: lmdb_zero::ReadTransaction::new(&env.env).unwrap() };
    }

    pub(in super) fn get<K, V>(&self, db: &LmdbDatabase<'env>, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        let access = self.txn.access();
        let result: Option<&[u8]> = access.get(&db.db, AsDatabaseKey::as_database_bytes(key).as_ref()).to_opt().unwrap();
        return Some(FromDatabaseValue::copy_from_database(result?).unwrap());
    }
}

#[derive(Debug)]
pub struct LmdbWriteTransaction<'env> {
    txn: lmdb_zero::WriteTransaction<'env>,
}

impl<'env> LmdbWriteTransaction<'env> {
    pub(in super) fn new(env: &'env LmdbEnvironment) -> Self {
        return LmdbWriteTransaction { txn: lmdb_zero::WriteTransaction::new(&env.env).unwrap() };
    }

    pub(in super) fn get<K, V>(&self, db: &LmdbDatabase<'env>, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        let access = self.txn.access();
        let result: Option<&[u8]> = access.get(&db.db, AsDatabaseKey::as_database_bytes(key).as_ref()).to_opt().unwrap();
        return Some(FromDatabaseValue::copy_from_database(result?).unwrap());
    }

    pub(in super) fn put<K, V>(&mut self, db: &LmdbDatabase, key: &K, value: &V) where K: AsDatabaseKey + ?Sized, V: IntoDatabaseValue + ?Sized {
        unsafe {
            let mut access = self.txn.access();
            let mut bytes: &mut [u8] = access.put_reserve_unsized(&db.db, AsDatabaseKey::as_database_bytes(key).as_ref(), IntoDatabaseValue::database_byte_size(value), lmdb_zero::put::Flags::empty()).unwrap();
            IntoDatabaseValue::copy_into_database(value, &mut bytes);
        }
    }

    pub(in super) fn remove<K>(&mut self, db: &LmdbDatabase, key: &K) where K: AsDatabaseKey + ?Sized {
        let mut access = self.txn.access();
        access.del_key(&db.db, AsDatabaseKey::as_database_bytes(key).as_ref()).to_opt().unwrap();
    }

    pub(in super) fn commit(self) {
        self.txn.commit().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_can_save_basic_objects() {
        let env = LmdbEnvironment::new("./test", 10485760, 1);
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
            tx.put(&db, "test", "one");
            assert_eq!(tx.get::<str, String>(&db, "test"), Some("one".to_string()));
            // Overwrite and read value.
            tx.put(&db, "test", "two");
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
            tx.put(&db, "test", "one");
            tx.abort();

            // Check aborted transaction.
            let tx = ReadTransaction::new(&env);
            assert!(tx.get::<str, String>(&db, "test").is_none());
        }

        env.drop_database().unwrap();
    }
}
