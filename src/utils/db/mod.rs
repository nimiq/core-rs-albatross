pub mod lmdb;
pub mod volatile;

use lmdb_zero;
use std::io;
use std::borrow::Cow;

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

#[derive(Debug)]
pub enum Environment {
    Volatile(volatile::VolatileEnvironment),
    Persistent(lmdb::LmdbEnvironment),
}

impl Environment {
    pub fn open_database(&self, name: String) -> Database {
        match *self {
            Environment::Volatile(ref env) => { return Database::Volatile(env.open_database(name)); }
            Environment::Persistent(ref env) => { return Database::Persistent(env.open_database(name)); }
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
    Volatile(volatile::VolatileDatabase),
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
pub enum ReadTransaction<'env> {
    Volatile(volatile::VolatileReadTransaction<'env>),
    Persistent(lmdb::LmdbReadTransaction<'env>),
}

impl<'env> ReadTransaction<'env> {
    pub fn new(env: &'env Environment) -> Self {
        match *env {
            Environment::Volatile(ref env) => { return ReadTransaction::Volatile(volatile::VolatileReadTransaction::new(env)); }
            Environment::Persistent(ref env) => { return ReadTransaction::Persistent(lmdb::LmdbReadTransaction::new(env)); }
        }
    }

    pub fn get<K, V>(&self, db: &Database, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        match *self {
            ReadTransaction::Volatile(ref txn) => { return txn.get(db.volatile().unwrap(), key); }
            ReadTransaction::Persistent(ref txn) => { return txn.get(db.persistent().unwrap(), key); }
        }
    }

    pub fn close(self) {}
}

#[derive(Debug)]
pub enum WriteTransaction<'env> {
    Volatile(volatile::VolatileWriteTransaction<'env>),
    Persistent(lmdb::LmdbWriteTransaction<'env>),
}

impl<'env> WriteTransaction<'env> {
    pub fn new(env: &'env Environment) -> Self {
        match *env {
            Environment::Volatile(ref env) => { return WriteTransaction::Volatile(volatile::VolatileWriteTransaction::new(env)); }
            Environment::Persistent(ref env) => { return WriteTransaction::Persistent(lmdb::LmdbWriteTransaction::new(env)); }
        }
    }

    pub fn get<K, V>(&self, db: &Database, key: &K) -> Option<V> where K: AsDatabaseKey + ?Sized, V: FromDatabaseValue {
        match *self {
            WriteTransaction::Volatile(ref txn) => { return txn.get(db.volatile().unwrap(), key); }
            WriteTransaction::Persistent(ref txn) => { return txn.get(db.persistent().unwrap(), key); }
        }
    }

    pub fn put<K, V>(&mut self, db: &Database, key: &K, value: &V) where K: AsDatabaseKey + ?Sized, V: IntoDatabaseValue + ?Sized {
        match *self {
            WriteTransaction::Volatile(ref mut txn) => { return txn.put(db.volatile().unwrap(), key, value); }
            WriteTransaction::Persistent(ref mut txn) => { return txn.put(db.persistent().unwrap(), key, value); }
        }
    }

    pub fn remove<K>(&mut self, db: &Database, key: &K) where K: AsDatabaseKey + ?Sized {
        match *self {
            WriteTransaction::Volatile(ref mut txn) => { return txn.remove(db.volatile().unwrap(), key); }
            WriteTransaction::Persistent(ref mut txn) => { return txn.remove(db.persistent().unwrap(), key); }
        }
    }

    pub fn commit(self) {
        match self {
            WriteTransaction::Volatile(txn) => { return txn.commit(); }
            WriteTransaction::Persistent(txn) => { return txn.commit(); }
        }
    }

    pub fn abort(self) {}
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
