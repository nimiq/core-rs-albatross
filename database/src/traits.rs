use std::borrow::Cow;
use std::ffi::CStr;
use std::io;
use std::mem;
use std::slice;

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
        Ok(u32::from_ne_bytes(bytes.try_into().expect("mismatch size")))
    }
}

impl FromDatabaseValue for u64 {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(u64::from_ne_bytes(bytes.try_into().expect("mismatch size")))
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

impl AsDatabaseBytes for str {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(&self.as_bytes())
    }
}

impl AsDatabaseBytes for CStr {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(&self.to_bytes())
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

macro_rules! as_db_bytes {
    ($typ:ident) => {
        impl AsDatabaseBytes for $typ {
            fn as_database_bytes(&self) -> Cow<[u8]> {
                unsafe {
                    Cow::Borrowed(slice::from_raw_parts(
                        self as *const $typ as *const u8,
                        mem::size_of::<$typ>(),
                    ))
                }
            }
        }
        impl AsDatabaseBytes for [$typ] {
            fn as_database_bytes(&self) -> Cow<[u8]> {
                unsafe {
                    Cow::Borrowed(slice::from_raw_parts(
                        self.as_ptr() as *const u8,
                        self.len() * mem::size_of::<$typ>(),
                    ))
                }
            }
        }
    };
}

as_db_bytes!(u8);
as_db_bytes!(u16);
as_db_bytes!(i16);
as_db_bytes!(u32);
as_db_bytes!(i32);
as_db_bytes!(u64);
as_db_bytes!(i64);
as_db_bytes!(f32);
as_db_bytes!(f64);
as_db_bytes!(char);
