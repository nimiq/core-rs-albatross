pub mod lmdb;
pub mod volatile;

use lmdb_zero;
use std::io;
use std::borrow::Cow;
use std::ops::Deref;

pub trait IntoDatabaseValue {
    fn database_byte_size(&self) -> usize;
    fn copy_into_database(&self, bytes: &mut [u8]);
}

pub trait FromDatabaseValue {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized;
}

pub trait AsDatabaseKey {
    fn as_database_bytes(&self) -> Cow<[u8]>;
}

bitflags! {
    #[derive(Default)]
    pub struct DatabaseFlags: u32 {
        /// Duplicate keys may be used in the database.
        const DUPLICATE_KEYS        = 0b00000001;
        /// This flag may only be used in combination with `DUPLICATE_KEYS`.
        /// This option tells the database that the values for this database are all the same size.
        const DUP_FIXED_SIZE_VALUES = 0b00000010;
        /// Keys are binary integers in native byte order and will be sorted as such.
        const USIZE_KEYS            = 0b00000100;
        /// This option specifies that duplicate data items are binary integers, similar to `USIZE_KEYS` keys.
        const DUP_USIZE_VALUES      = 0b00001000;
    }
}

#[derive(Debug)]
pub enum Environment {
    Volatile(volatile::VolatileEnvironment),
    Persistent(lmdb::LmdbEnvironment),
}

impl Environment {
    pub fn open_database(&self, name: String) -> Database {
        match *self {
            Environment::Volatile(ref env) => { return Database::Volatile(env.open_database(name, Default::default())); }
            Environment::Persistent(ref env) => { return Database::Persistent(env.open_database(name, Default::default())); }
        }
    }

    pub fn open_database_with_flags(&self, name: String, flags: DatabaseFlags) -> Database {
        match *self {
            Environment::Volatile(ref env) => { return Database::Volatile(env.open_database(name, flags)); }
            Environment::Persistent(ref env) => { return Database::Persistent(env.open_database(name, flags)); }
        }
    }

    pub fn close(self) {}

    pub fn drop_database(self) -> io::Result<()> {
        match self {
            Environment::Volatile(env) => { return Ok(()); }
            Environment::Persistent(env) => { return env.drop_database(); }
        }
    }
}

#[derive(Debug)]
pub enum Database<'env> {
    Volatile(volatile::VolatileDatabase<'env>),
    Persistent(lmdb::LmdbDatabase<'env>),
}

impl<'env> Database<'env> {
    fn volatile(&self) -> Option<&volatile::VolatileDatabase> {
        if let Database::Volatile(ref db) = self {
            return Some(db);
        }
        return None;
    }

    fn persistent(&self) -> Option<&lmdb::LmdbDatabase> {
        if let Database::Persistent(ref db) = self {
            return Some(db);
        }
        return None;
    }
}

#[derive(Debug)]
pub enum Transaction<'env> {
    VolatileRead(volatile::VolatileReadTransaction<'env>),
    VolatileWrite(volatile::VolatileWriteTransaction<'env>),
    PersistentRead(lmdb::LmdbReadTransaction<'env>),
    PersistentWrite(lmdb::LmdbWriteTransaction<'env>),
}

impl<'env> Transaction<'env> {
    pub fn get<K, V>(&self, db: &Database, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        match *self {
            Transaction::VolatileRead(ref txn) => { return txn.get(db.volatile().unwrap(), key); }
            Transaction::VolatileWrite(ref txn) => { return txn.get(db.volatile().unwrap(), key); }
            Transaction::PersistentRead(ref txn) => { return txn.get(db.persistent().unwrap(), key); }
            Transaction::PersistentWrite(ref txn) => { return txn.get(db.persistent().unwrap(), key); }
        }
    }
}

#[derive(Debug)]
pub struct ReadTransaction<'env>(Transaction<'env>);

impl<'env> ReadTransaction<'env> {
    pub fn new(env: &'env Environment) -> Self {
        match *env {
            Environment::Volatile(ref env) => { return ReadTransaction(Transaction::VolatileRead(volatile::VolatileReadTransaction::new(env))); }
            Environment::Persistent(ref env) => { return ReadTransaction(Transaction::PersistentRead(lmdb::LmdbReadTransaction::new(env))); }
        }
    }

    pub fn get<K, V>(&self, db: &Database, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        return self.0.get(db, key);
    }

    pub fn close(self) {}
}

impl<'env> Deref for ReadTransaction<'env> {
    type Target = Transaction<'env>;

    fn deref(&self) -> &Transaction<'env> {
        return &self.0;
    }
}

#[derive(Debug)]
pub struct WriteTransaction<'env>(Transaction<'env>);

impl<'env> WriteTransaction<'env> {
    pub fn new(env: &'env Environment) -> Self {
        match *env {
            Environment::Volatile(ref env) => { return WriteTransaction(Transaction::VolatileWrite(volatile::VolatileWriteTransaction::new(env))); }
            Environment::Persistent(ref env) => { return WriteTransaction(Transaction::PersistentWrite(lmdb::LmdbWriteTransaction::new(env))); }
        }
    }

    pub fn get<K, V>(&self, db: &Database, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        return self.0.get(db, key);
    }

    pub fn put<K, V>(&mut self, db: &Database, key: &K, value: &V) where K: AsDatabaseKey + ?Sized, V: IntoDatabaseValue + ?Sized {
        match self.0 {
            Transaction::VolatileWrite(ref mut txn) => { return txn.put(db.volatile().unwrap(), key, value); }
            Transaction::PersistentWrite(ref mut txn) => { return txn.put(db.persistent().unwrap(), key, value); }
            _ => { unreachable!(); }
        }
    }

    pub fn remove<K>(&mut self, db: &Database, key: &K) where K: AsDatabaseKey + ?Sized {
        match self.0 {
            Transaction::VolatileWrite(ref mut txn) => { return txn.remove(db.volatile().unwrap(), key); }
            Transaction::PersistentWrite(ref mut txn) => { return txn.remove(db.persistent().unwrap(), key); }
            _ => { unreachable!(); }
        }
    }

    pub fn commit(self) {
        match self.0 {
            Transaction::VolatileWrite(txn) => { return txn.commit(); }
            Transaction::PersistentWrite(txn) => { return txn.commit(); }
            _ => { unreachable!(); }
        }
    }

    pub fn abort(self) {}
}

impl<'env> Deref for WriteTransaction<'env> {
    type Target = Transaction<'env>;

    fn deref(&self) -> &Transaction<'env> {
        return &self.0;
    }
}

impl IntoDatabaseValue for [u8] {
    fn database_byte_size(&self) -> usize {
        return self.len();
    }

    fn copy_into_database(&self, bytes: &mut [u8]) {
        bytes.copy_from_slice(self);
    }
}

impl FromDatabaseValue for Vec<u8> {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        return Ok(bytes.to_vec());
    }
}

impl IntoDatabaseValue for str {
    fn database_byte_size(&self) -> usize {
        return self.len();
    }

    fn copy_into_database(&self, bytes: &mut [u8]) {
        println!("{} {}", bytes.len(), self.len());
        bytes.copy_from_slice(self.as_bytes());
    }
}

impl FromDatabaseValue for String {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        return Ok(String::from_utf8(bytes.to_vec()).unwrap());
    }
}

impl<T> AsDatabaseKey for T
    where T: lmdb_zero::traits::AsLmdbBytes + ?Sized {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        return Cow::Borrowed(self.as_lmdb_bytes());
    }
}
