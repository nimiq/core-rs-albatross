use std::borrow::Cow;
use std::ffi::CStr;
use std::io;

use lmdb_zero::traits::AsLmdbBytes;

pub trait IntoDatabaseValue {
    fn database_byte_size(&self) -> usize;
    fn copy_into_database(&self, bytes: &mut [u8]);
}

pub trait FromDatabaseValue {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized;
}

pub trait AsDatabaseBytes {
    fn as_database_bytes(&self) -> Cow<[u8]>;
}

// Trait implementations
impl IntoDatabaseValue for [u8] {
    fn database_byte_size(&self) -> usize {
        self.len()
    }

    fn copy_into_database(&self, bytes: &mut [u8]) {
        bytes.copy_from_slice(self);
    }
}

impl IntoDatabaseValue for str {
    fn database_byte_size(&self) -> usize {
        self.len()
    }

    fn copy_into_database(&self, bytes: &mut [u8]) {
        bytes.copy_from_slice(self.as_bytes());
    }
}

impl FromDatabaseValue for String {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(String::from_utf8(bytes.to_vec()).unwrap())
    }
}

impl FromDatabaseValue for u32 {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let lmdb_result: Result<&lmdb_zero::Unaligned<u32>, String> =
            lmdb_zero::traits::FromLmdbBytes::from_lmdb_bytes(bytes);
        Ok(lmdb_result
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .get())
    }
}

impl FromDatabaseValue for u64 {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let lmdb_result: Result<&lmdb_zero::Unaligned<u64>, String> =
            lmdb_zero::traits::FromLmdbBytes::from_lmdb_bytes(bytes);
        Ok(lmdb_result
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .get())
    }
}

impl FromDatabaseValue for Vec<u8> {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(bytes.to_vec())
    }
}

impl AsDatabaseBytes for Vec<u8> {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(&self[..])
    }
}

// Conflicting implementation:
//impl<T> FromDatabaseValue for T
//    where T: lmdb_zero::traits::FromLmdbBytes + ?Sized {
//    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
//        Ok(lmdb_zero::traits::FromLmdbBytes::from_lmdb_bytes(bytes).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?.to_owned())
//    }
//}

//impl<T> AsDatabaseBytes for T
//    where T: lmdb_zero::traits::AsLmdbBytes + ?Sized {
//    fn as_database_bytes(&self) -> Cow<[u8]> {
//        return Cow::Borrowed(self.as_lmdb_bytes());
//    }
//}

macro_rules! as_lmdb_bytes {
    ($t: ty) => {
        impl AsDatabaseBytes for $t {
            fn as_database_bytes(&self) -> Cow<[u8]> {
                return Cow::Borrowed(self.as_lmdb_bytes());
            }
        }
    };
}

as_lmdb_bytes!(u8);
as_lmdb_bytes!(u16);
as_lmdb_bytes!(i16);
as_lmdb_bytes!(u32);
as_lmdb_bytes!(i32);
as_lmdb_bytes!(u64);
as_lmdb_bytes!(i64);
as_lmdb_bytes!(f32);
as_lmdb_bytes!(f64);
as_lmdb_bytes!(str);
as_lmdb_bytes!(CStr);
as_lmdb_bytes!(char);

as_lmdb_bytes!([u8]);
as_lmdb_bytes!([u16]);
as_lmdb_bytes!([i16]);
as_lmdb_bytes!([u32]);
as_lmdb_bytes!([i32]);
as_lmdb_bytes!([u64]);
as_lmdb_bytes!([i64]);
as_lmdb_bytes!([f32]);
as_lmdb_bytes!([f64]);
as_lmdb_bytes!([char]);
