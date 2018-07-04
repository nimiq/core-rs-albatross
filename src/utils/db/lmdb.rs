use super::*;
use lmdb_zero::traits::LmdbResultExt;
use std::fs;
use fs2;
use rand::distributions::{IndependentSample, Range};
use rand;
use std::cmp;
use lmdb_zero;

#[derive(Debug)]
pub struct LmdbEnvironment {
    env: lmdb_zero::Environment,
}

impl LmdbEnvironment {
    pub fn new(path: &str, size: usize, max_dbs: u32, flags: lmdb_zero::open::Flags) -> Environment {
        fs::create_dir_all(path).unwrap();

        let mut env = lmdb_zero::EnvBuilder::new().unwrap();
        env.set_maxdbs(max_dbs).unwrap();
        let env = unsafe {
            env.open(path, flags, 0o600).unwrap()
        };

        let info = env.info().unwrap();
        let cur_mapsize = info.mapsize;
        if cur_mapsize < size {
            unsafe { env.set_mapsize(size).unwrap() };
            let info = env.info().unwrap();
            let cur_mapsize = info.mapsize;
            info!("LMDB memory map size: {}", cur_mapsize);
        }

        let lmdb = LmdbEnvironment { env };
        if lmdb.need_resize(0) {
            info!("LMDB memory needs to be resized.");
            lmdb.do_resize(0);
        }

        return Environment::Persistent(lmdb);
    }

    pub(in super) fn open_database<'env>(&'env self, name: String) -> LmdbDatabase<'env> {
        return LmdbDatabase { db: lmdb_zero::Database::open(&self.env, Some(&name), &lmdb_zero::DatabaseOptions::new(lmdb_zero::db::CREATE)).unwrap() };
    }

    pub(in super) fn drop_database(self) -> io::Result<()> {
        return fs::remove_dir_all(self.path().as_ref());
    }

    fn path(&self) -> Cow<str> {
        return self.env.path().unwrap().to_string_lossy();
    }

    pub fn do_resize(&self, increase_size: usize) {
        let add_size: usize = cmp::max(1 << 30, increase_size); // TODO: use increase_size

        let available_space = fs2::available_space(self.path().as_ref());
        match available_space {
            Ok(available_space) => {
                let available_space = available_space as usize;
                // Check disk capacity.
                if available_space < add_size {
                    error!("Insufficient free space to extend database: {} MB available, {} MB needed.", available_space >> 20, add_size >> 20);
                    return;
                }
            },
            Err(e) => {
                warn!("Unable to query free disk space:\n{:?}", e);
            },
        }

        let info = self.env.info().unwrap();
        let stat = self.env.stat().unwrap();

        let mut new_mapsize = info.mapsize + add_size;
        new_mapsize += new_mapsize % (stat.psize as usize);

        // TODO: Should we handle the error?
        unsafe {
            self.env.set_mapsize(new_mapsize).unwrap();
        }

        info!("LMDB Mapsize increased. Old: {} MiB, New: {} MiB", info.mapsize / (1024 * 1024), new_mapsize / (1024 * 1024));
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

        let between = Range::new(0.6, 0.9);
        let mut rng = rand::thread_rng();
        let resize_percent = between.ind_sample(&mut rng);

        if (size_used as f64) / (info.mapsize as f64) > resize_percent {
            info!("DB resize (percent-based)");
            info!("DB map size: {}", info.mapsize);
            info!("Space used: {}", size_used);
            info!("Space remaining: {}", info.mapsize - size_used);
            info!("Percent used: {:.2}", (size_used as f64) / (info.mapsize as f64));
            return true;
        }

        return false;
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
        // Check for enough space before every write transaction.
        if env.need_resize(0) {
            env.do_resize(0);
        }
        return LmdbWriteTransaction { txn: lmdb_zero::WriteTransaction::new(&env.env).unwrap() };
    }

    pub(in super) fn get<K, V>(&self, db: &LmdbDatabase<'env>, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        let access = self.txn.access();
        let result: Option<&[u8]> = access.get(&db.db, AsDatabaseKey::as_database_bytes(key).as_ref()).to_opt().unwrap();
        return Some(FromDatabaseValue::copy_from_database(result?).unwrap());
    }

    pub(in super) fn put<K, V>(&mut self, db: &LmdbDatabase, key: &K, value: &V) where K: AsDatabaseKey + ?Sized, V: IntoDatabaseValue + ?Sized {
        let key = AsDatabaseKey::as_database_bytes(key);
        let value_size = IntoDatabaseValue::database_byte_size(value);
        unsafe {
            let mut access = self.txn.access();
            let mut bytes: &mut [u8] = access.put_reserve_unsized(&db.db, key.as_ref(), value_size, lmdb_zero::put::Flags::empty()).unwrap();
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
        let env = LmdbEnvironment::new("./test", 0, 1, lmdb_zero::open::Flags::empty());
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
        let env = LmdbEnvironment::new("./test2", 0, 1, lmdb_zero::open::NOTLS);
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
