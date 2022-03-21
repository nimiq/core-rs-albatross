#[macro_use]
extern crate log;

use std::borrow::Cow;
use std::error;
use std::fmt;
use std::io;
use std::ops::Deref;

use bitflags::bitflags;

use crate::cursor::{ReadCursor, WriteCursor as WriteCursorTrait};
pub use crate::traits::{AsDatabaseBytes, FromDatabaseValue, IntoDatabaseValue};

#[macro_use]
pub mod cursor;
pub mod mdbx;
pub mod traits;
pub mod volatile;

#[derive(Debug)]
pub enum Error {
    CreateDirectory(io::Error),
    Mdbx(libmdbx::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use Error::*;
        match self {
            CreateDirectory(e) => write!(f, "couldn't create directory for DB: {}", e),
            Mdbx(e) => e.fmt(f),
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        use Error::*;
        Some(match self {
            CreateDirectory(e) => e,
            Mdbx(e) => e,
        })
    }
}

impl From<libmdbx::Error> for Error {
    fn from(e: libmdbx::Error) -> Error {
        Error::Mdbx(e)
    }
}

bitflags! {
    #[derive(Default)]
    pub struct DatabaseFlags: u32 {
        /// Duplicate keys may be used in the database.
        const DUPLICATE_KEYS        = 0b0000_0001;
        /// This flag may only be used in combination with `DUPLICATE_KEYS`.
        /// This option tells the database that the values for this database are all the same size.
        const DUP_FIXED_SIZE_VALUES = 0b0000_0010;
        /// Keys are binary integers in native byte order and will be sorted as such
        /// (`std::os::raw::c_uint`, i.e. most likely `u32`).
        const UINT_KEYS             = 0b0000_0100;
    }
}

#[derive(Clone, Debug)]
pub enum Environment {
    Volatile(volatile::VolatileEnvironment),
    Persistent(mdbx::MdbxEnvironment),
}

impl Environment {
    pub fn open_database(&self, name: String) -> Database {
        match *self {
            Environment::Volatile(ref env) => {
                Database::Volatile(env.open_database(name, Default::default()))
            }
            Environment::Persistent(ref env) => {
                Database::Persistent(env.open_database(name, Default::default()))
            }
        }
    }

    pub fn open_database_with_flags(&self, name: String, flags: DatabaseFlags) -> Database {
        match *self {
            Environment::Volatile(ref env) => Database::Volatile(env.open_database(name, flags)),
            Environment::Persistent(ref env) => {
                Database::Persistent(env.open_database(name, flags))
            }
        }
    }

    pub fn close(self) {}
}

#[derive(Debug)]
pub enum Database {
    Volatile(volatile::VolatileDatabase),
    Persistent(mdbx::MdbxDatabase),
}

impl Database {
    fn volatile(&self) -> Option<&volatile::VolatileDatabase> {
        if let Database::Volatile(ref db) = self {
            return Some(db);
        }
        None
    }

    fn persistent(&self) -> Option<&mdbx::MdbxDatabase> {
        match self {
            Database::Persistent(ref db) => Some(db),
            Database::Volatile(ref db) => Some(db.as_mdbx()),
        }
    }
}

#[derive(Debug)]
pub enum Transaction<'env> {
    VolatileRead(volatile::VolatileReadTransaction<'env>),
    VolatileWrite(volatile::VolatileWriteTransaction<'env>),
    PersistentRead(mdbx::MdbxReadTransaction<'env>),
    PersistentWrite(mdbx::MdbxWriteTransaction<'env>),
}

impl<'env> Transaction<'env> {
    pub fn get<K, V>(&self, db: &Database, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        match *self {
            Transaction::VolatileRead(ref txn) => txn.get(db.volatile().unwrap(), key),
            Transaction::VolatileWrite(ref txn) => txn.get(db.volatile().unwrap(), key),
            Transaction::PersistentRead(ref txn) => txn.get(db.persistent().unwrap(), key),
            Transaction::PersistentWrite(ref txn) => txn.get(db.persistent().unwrap(), key),
        }
    }

    pub fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> Cursor<'txn> {
        match *self {
            Transaction::VolatileRead(ref txn) => Cursor::VolatileCursor(txn.cursor(db)),
            Transaction::VolatileWrite(ref txn) => Cursor::VolatileCursor(txn.cursor(db)),
            Transaction::PersistentRead(ref txn) => Cursor::PersistentCursor(txn.cursor(db)),
            Transaction::PersistentWrite(ref txn) => Cursor::PersistentCursor(txn.cursor(db)),
        }
    }
}

#[derive(Debug)]
pub struct ReadTransaction<'env>(Transaction<'env>);

impl<'env> ReadTransaction<'env> {
    pub fn new(env: &'env Environment) -> Self {
        match *env {
            Environment::Volatile(ref env) => ReadTransaction(Transaction::VolatileRead(
                volatile::VolatileReadTransaction::new(env),
            )),
            Environment::Persistent(ref env) => ReadTransaction(Transaction::PersistentRead(
                mdbx::MdbxReadTransaction::new(env),
            )),
        }
    }

    pub fn get<K, V>(&self, db: &Database, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        self.0.get(db, key)
    }

    pub fn close(self) {}

    pub fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> Cursor<'txn> {
        self.0.cursor(db)
    }
}

impl<'env> Deref for ReadTransaction<'env> {
    type Target = Transaction<'env>;

    fn deref(&self) -> &Transaction<'env> {
        &self.0
    }
}

#[derive(Debug)]
pub struct WriteTransaction<'env>(Transaction<'env>);

impl<'env> WriteTransaction<'env> {
    pub fn new(env: &'env Environment) -> Self {
        match *env {
            Environment::Volatile(ref env) => WriteTransaction(Transaction::VolatileWrite(
                volatile::VolatileWriteTransaction::new(env),
            )),
            Environment::Persistent(ref env) => WriteTransaction(Transaction::PersistentWrite(
                mdbx::MdbxWriteTransaction::new(env),
            )),
        }
    }

    pub fn get<K, V>(&self, db: &Database, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        self.0.get(db, key)
    }

    /// Puts a key/value pair into the database by copying it into a reserved space in the database.
    /// This works best for values that need to be serialized into the reserved space.
    /// This method will panic when called on a database with duplicate keys!
    pub fn put_reserve<K, V>(&mut self, db: &Database, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: IntoDatabaseValue + ?Sized,
    {
        match self.0 {
            Transaction::VolatileWrite(ref mut txn) => {
                txn.put_reserve(db.volatile().unwrap(), key, value)
            }
            Transaction::PersistentWrite(ref mut txn) => {
                txn.put_reserve(db.persistent().unwrap(), key, value)
            }
            _ => {
                unreachable!();
            }
        }
    }

    /// Puts a key/value pair into the database by passing a reference to a byte slice.
    /// This is more efficient than `put_reserve` if no serialization is needed,
    /// and the existing value can be immediately written into the database.
    /// This also works with duplicate key databases.
    pub fn put<K, V>(&mut self, db: &Database, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        match self.0 {
            Transaction::VolatileWrite(ref mut txn) => txn.put(db.volatile().unwrap(), key, value),
            Transaction::PersistentWrite(ref mut txn) => {
                txn.put(db.persistent().unwrap(), key, value)
            }
            _ => {
                unreachable!();
            }
        }
    }

    pub fn remove<K>(&mut self, db: &Database, key: &K)
    where
        K: AsDatabaseBytes + ?Sized,
    {
        match self.0 {
            Transaction::VolatileWrite(ref mut txn) => txn.remove(db.volatile().unwrap(), key),
            Transaction::PersistentWrite(ref mut txn) => txn.remove(db.persistent().unwrap(), key),
            _ => {
                unreachable!();
            }
        }
    }

    pub fn remove_item<K, V>(&mut self, db: &Database, key: &K, value: &V)
    where
        K: AsDatabaseBytes + ?Sized,
        V: AsDatabaseBytes + ?Sized,
    {
        match self.0 {
            Transaction::VolatileWrite(ref mut txn) => {
                txn.remove_item(db.volatile().unwrap(), key, value)
            }
            Transaction::PersistentWrite(ref mut txn) => {
                txn.remove_item(db.persistent().unwrap(), key, value)
            }
            _ => {
                unreachable!();
            }
        }
    }

    pub fn commit(self) {
        match self.0 {
            Transaction::VolatileWrite(txn) => txn.commit(),
            Transaction::PersistentWrite(txn) => txn.commit(),
            _ => {
                unreachable!();
            }
        }
    }

    pub fn abort(self) {}

    pub fn cursor<'txn, 'db>(&'txn self, db: &'db Database) -> Cursor<'txn> {
        self.0.cursor(db)
    }

    pub fn write_cursor<'txn, 'db>(&'txn self, db: &'db Database) -> WriteCursor<'txn> {
        match self.0 {
            Transaction::VolatileWrite(ref txn) => {
                WriteCursor::VolatileCursor(txn.write_cursor(db))
            }
            Transaction::PersistentWrite(ref txn) => {
                WriteCursor::PersistentCursor(txn.write_cursor(db))
            }
            _ => unreachable!(),
        }
    }
}

impl<'env> Deref for WriteTransaction<'env> {
    type Target = Transaction<'env>;

    fn deref(&self) -> &Transaction<'env> {
        &self.0
    }
}

pub enum Cursor<'txn> {
    VolatileCursor(volatile::VolatileCursor<'txn>),
    PersistentCursor(mdbx::MdbxCursor<'txn>),
}

pub enum WriteCursor<'txn> {
    VolatileCursor(volatile::VolatileWriteCursor<'txn>),
    PersistentCursor(mdbx::MdbxWriteCursor<'txn>),
}

macro_rules! gen_cursor_match {
    ($self: ident, $f: ident, $t: ident) => {
        match $self {
            $t::PersistentCursor(ref mut cursor) => cursor.$f(),
            $t::VolatileCursor(ref mut cursor) => cursor.$f(),
        }
    };
    ($self: ident, $f: ident, $k: expr, $t: ident) => {
        match $self {
            $t::PersistentCursor(ref mut cursor) => cursor.$f($k),
            $t::VolatileCursor(ref mut cursor) => cursor.$f($k),
        }
    };
    ($self: ident, $f: ident, $k: expr, $v: expr, $t: ident) => {
        match $self {
            $t::PersistentCursor(ref mut cursor) => cursor.$f($k, $v),
            $t::VolatileCursor(ref mut cursor) => cursor.$f($k, $v),
        }
    };
}

impl<'txn, 'db> ReadCursor for Cursor<'txn> {
    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, first, Cursor)
    }

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, first_duplicate, Cursor)
    }

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, last, Cursor)
    }

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, last_duplicate, Cursor)
    }

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, get_current, Cursor)
    }

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, next, Cursor)
    }

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, next_duplicate, Cursor)
    }

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, next_no_duplicate, Cursor)
    }

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, prev, Cursor)
    }

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, prev_duplicate, Cursor)
    }

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, prev_no_duplicate, Cursor)
    }

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, seek_key, key, Cursor)
    }

    fn seek_key_both<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, seek_key_both, key, Cursor)
    }

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, seek_range_key, key, Cursor)
    }

    fn count_duplicates(&mut self) -> usize {
        gen_cursor_match!(self, count_duplicates, Cursor)
    }
}

impl<'txn, 'db> ReadCursor for WriteCursor<'txn> {
    fn first<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, first, WriteCursor)
    }

    fn first_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, first_duplicate, WriteCursor)
    }

    fn last<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, last, WriteCursor)
    }

    fn last_duplicate<V>(&mut self) -> Option<V>
    where
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, last_duplicate, WriteCursor)
    }

    fn get_current<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, get_current, WriteCursor)
    }

    fn next<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, next, WriteCursor)
    }

    fn next_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, next_duplicate, WriteCursor)
    }

    fn next_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, next_no_duplicate, WriteCursor)
    }

    fn prev<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, prev, WriteCursor)
    }

    fn prev_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, prev_duplicate, WriteCursor)
    }

    fn prev_no_duplicate<K, V>(&mut self) -> Option<(K, V)>
    where
        K: FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, prev_no_duplicate, WriteCursor)
    }

    fn seek_key<K, V>(&mut self, key: &K) -> Option<V>
    where
        K: AsDatabaseBytes + ?Sized,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, seek_key, key, WriteCursor)
    }

    fn seek_key_both<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, seek_key_both, key, WriteCursor)
    }

    fn seek_range_key<K, V>(&mut self, key: &K) -> Option<(K, V)>
    where
        K: AsDatabaseBytes + FromDatabaseValue,
        V: FromDatabaseValue,
    {
        gen_cursor_match!(self, seek_range_key, key, WriteCursor)
    }

    fn count_duplicates(&mut self) -> usize {
        gen_cursor_match!(self, count_duplicates, WriteCursor)
    }
}

impl<'txn, 'db> WriteCursorTrait for WriteCursor<'txn> {
    fn remove(&mut self) {
        gen_cursor_match!(self, remove, WriteCursor)
    }
}
