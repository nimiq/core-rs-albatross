use super::*;
use lmdb_zero::traits::LmdbResultExt;
use std::fs;
use fs2;
use rand::distributions::{IndependentSample, Range};
use rand;
use std::cmp;
use lmdb_zero;
use parking_lot;
use std::fmt;

#[derive(Debug)]
pub struct LmdbEnvironment {
    env: lmdb_zero::Environment,
    creation_gate: parking_lot::RwLock<()>,
}

impl LmdbEnvironment {
    pub fn new(path: &str, size: usize, max_dbs: u32, flags: lmdb_zero::open::Flags) -> Result<Environment, lmdb_zero::Error> {
        return Ok(Environment::Persistent(LmdbEnvironment::new_lmdb_environment(path, size, max_dbs, flags)?));
    }

    pub(in super) fn new_lmdb_environment(path: &str, size: usize, max_dbs: u32, flags: lmdb_zero::open::Flags) -> Result<Self, lmdb_zero::Error> {
        fs::create_dir_all(path).unwrap();

        let mut env = lmdb_zero::EnvBuilder::new()?;
        env.set_maxdbs(max_dbs)?;
        let env = unsafe {
            env.open(path, flags, 0o600)?
        };

        let info = env.info()?;
        let cur_mapsize = info.mapsize;
        if cur_mapsize < size {
            unsafe { env.set_mapsize(size)? };
            let info = env.info()?;
            let cur_mapsize = info.mapsize;
            info!("LMDB memory map size: {}", cur_mapsize);
        }

        let lmdb = LmdbEnvironment { env, creation_gate: parking_lot::RwLock::new(()) };
        if lmdb.need_resize(0) {
            info!("LMDB memory needs to be resized.");
            lmdb.do_resize(0);
        }

        return Ok(lmdb);
    }

    pub(in super) fn open_database<'env>(&'env self, name: String, flags: DatabaseFlags) -> LmdbDatabase<'env> {
        // This is an implicit transaction, so take the lock first.
        let guard = self.creation_gate.read();
        let mut db_flags = lmdb_zero::db::CREATE;

        // Translate flags.
        if flags.contains(DatabaseFlags::DUPLICATE_KEYS) {
            db_flags.insert(lmdb_zero::db::DUPSORT);

            if flags.contains(DatabaseFlags::DUP_FIXED_SIZE_VALUES) {
                db_flags.insert(lmdb_zero::db::DUPFIXED);
            }

            if flags.contains(DatabaseFlags::DUP_USIZE_VALUES) {
                db_flags.insert(lmdb_zero::db::INTEGERDUP);
            }
        }
        if flags.contains(DatabaseFlags::USIZE_KEYS) {
            db_flags.insert(lmdb_zero::db::INTEGERKEY);
        }

        return LmdbDatabase { db: lmdb_zero::Database::open(&self.env, Some(&name), &lmdb_zero::DatabaseOptions::new(db_flags)).unwrap() };
    }

    pub(in super) fn drop_database(self) -> io::Result<()> {
        return fs::remove_dir_all(self.path().as_ref());
    }

    fn path(&self) -> Cow<str> {
        return self.env.path().unwrap().to_string_lossy();
    }

    pub fn do_resize(&self, increase_size: usize) {
        // Lock creation of new transactions until resize is finished.
        let guard = self.creation_gate.write();
        let add_size: usize = cmp::max(1 << 30, increase_size);

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

pub struct LmdbReadTransaction<'env> {
    txn: lmdb_zero::ReadTransaction<'env>,
    guard: parking_lot::RwLockReadGuard<'env, ()>,
}

impl<'env> LmdbReadTransaction<'env> {
    pub(in super) fn new(env: &'env LmdbEnvironment) -> Self {
        // This is an implicit transaction, so take the lock first.
        let guard = env.creation_gate.read();
        return LmdbReadTransaction { txn: lmdb_zero::ReadTransaction::new(&env.env).unwrap(), guard };
    }

    pub(in super) fn get<K, V>(&self, db: &LmdbDatabase<'env>, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        let access = self.txn.access();
        let result: Option<&[u8]> = access.get(&db.db, AsDatabaseKey::as_database_bytes(key).as_ref()).to_opt().unwrap();
        return Some(FromDatabaseValue::copy_from_database(result?).unwrap());
    }
}

impl<'env> fmt::Debug for LmdbReadTransaction<'env> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "LmdbReadTransaction {{ txn: {:?} }}", self.txn)
    }
}

pub struct LmdbWriteTransaction<'env> {
    txn: lmdb_zero::WriteTransaction<'env>,
    guard: parking_lot::RwLockReadGuard<'env, ()>,
}

impl<'env> LmdbWriteTransaction<'env> {
    pub(in super) fn new(env: &'env LmdbEnvironment) -> Self {
        // Check for enough space before every write transaction.
        if env.need_resize(0) {
            env.do_resize(0);
        }
        let guard = env.creation_gate.read();
        return LmdbWriteTransaction { txn: lmdb_zero::WriteTransaction::new(&env.env).unwrap(), guard };
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

impl<'env> fmt::Debug for LmdbWriteTransaction<'env> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "LmdbWriteTransaction {{ txn: {:?} }}", self.txn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_can_save_basic_objects() {
        let env = LmdbEnvironment::new("./test", 0, 1, lmdb_zero::open::Flags::empty()).unwrap();
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
        let env = LmdbEnvironment::new("./test2", 0, 1, lmdb_zero::open::NOTLS).unwrap();
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
