use super::*;
use super::lmdb::*;
use tempdir::TempDir;
use std::io;

#[derive(Debug)]
pub struct VolatileEnvironment {
    temp_dir: TempDir,
    env: LmdbEnvironment,
}

#[derive(Debug)]
pub enum VolatileDatabaseError {
    IoError(io::Error),
    LmdbError(lmdb_zero::Error),
}

impl VolatileEnvironment {
    pub fn new(max_dbs: u32) -> Result<Environment, VolatileDatabaseError> {
        let temp_dir = TempDir::new("volatile-core").map_err(|e| VolatileDatabaseError::IoError(e))?;
        let path = temp_dir.path().to_str().ok_or(VolatileDatabaseError::IoError(io::Error::new(io::ErrorKind::InvalidInput, "Path cannot be converted into a string.")))?.to_string();
        return Ok(Environment::Volatile(VolatileEnvironment {
            temp_dir,
            env: LmdbEnvironment::new_lmdb_environment(&path, 0, max_dbs, lmdb_zero::open::NOSYNC | lmdb_zero::open::WRITEMAP).map_err(|e| VolatileDatabaseError::LmdbError(e))?,
        }));
    }

    pub fn new_with_lmdb_flags(max_dbs: u32, flags: lmdb_zero::open::Flags) -> Result<Environment, VolatileDatabaseError> {
        let temp_dir = TempDir::new("volatile-core").map_err(|e| VolatileDatabaseError::IoError(e))?;
        let path = temp_dir.path().to_str().ok_or(VolatileDatabaseError::IoError(io::Error::new(io::ErrorKind::InvalidInput, "Path cannot be converted into a string.")))?.to_string();
        return Ok(Environment::Volatile(VolatileEnvironment {
            temp_dir,
            env: LmdbEnvironment::new_lmdb_environment(&path, 0, max_dbs, flags | lmdb_zero::open::NOSYNC | lmdb_zero::open::WRITEMAP).map_err(|e| VolatileDatabaseError::LmdbError(e))?,
        }));
    }

    pub(in super) fn open_database<'env>(&'env self, name: String, flags: DatabaseFlags) -> VolatileDatabase<'env> {
        return VolatileDatabase(self.env.open_database(name, flags));
    }

    pub(in super) fn drop_database(self) -> io::Result<()> {
        return Ok(());
    }
}

#[derive(Debug)]
pub struct VolatileDatabase<'env>(LmdbDatabase<'env>);


#[derive(Debug)]
pub struct VolatileReadTransaction<'env>(LmdbReadTransaction<'env>);

impl<'env> VolatileReadTransaction<'env> {
    pub(in super) fn new(env: &'env VolatileEnvironment) -> Self {
        return VolatileReadTransaction(LmdbReadTransaction::new(&env.env));
    }

    pub(in super) fn get<K, V>(&self, db: &VolatileDatabase, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        self.0.get(&db.0, key)
    }
}

#[derive(Debug)]
pub struct VolatileWriteTransaction<'env>(LmdbWriteTransaction<'env>);

impl<'env> VolatileWriteTransaction<'env> {
    pub(in super) fn new(env: &'env VolatileEnvironment) -> Self {
        return VolatileWriteTransaction(LmdbWriteTransaction::new(&env.env));
    }

    pub(in super) fn get<K, V>(&self, db: &VolatileDatabase, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        self.0.get(&db.0, key)
    }

    pub(in super) fn put<K, V>(&mut self, db: &VolatileDatabase, key: &K, value: &V) where K: AsDatabaseKey + ?Sized, V: IntoDatabaseValue + ?Sized {
        self.0.put(&db.0, key, value)
    }

    pub(in super) fn remove<K>(&mut self, db: &VolatileDatabase, key: &K) where K: AsDatabaseKey + ?Sized {
        self.0.remove(&db.0, key)
    }

    pub(in super) fn commit(self) {
        self.0.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn isolation_test() {
        let env = VolatileEnvironment::new_with_lmdb_flags( 1, lmdb_zero::open::NOTLS).unwrap();
        {
            let db = env.open_database("test".to_string());

            // Read non-existent value.
            let tx = ReadTransaction::new(&env);
            assert!(tx.get::<str, String>(&db, "test").is_none());

            // WriteTransaction.
            let mut txw = WriteTransaction::new(&env);
            assert!(txw.get::<str, String>(&db, "test").is_none());
            txw.put(&db, "test", "one");
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
}
