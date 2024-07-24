use std::{borrow::Cow, ffi::CStr, io, mem, slice};

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
        Cow::Borrowed(self.as_bytes())
    }
}

impl AsDatabaseBytes for CStr {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(self.to_bytes())
    }
}

macro_rules! as_db_bytes {
    ($typ:ident) => {
        impl AsDatabaseBytes for $typ {
            fn as_database_bytes(&self) -> Cow<[u8]> {
                unsafe {
                    #[allow(clippy::size_of_in_element_count)]
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
                    #[allow(clippy::size_of_in_element_count)]
                    Cow::Borrowed(slice::from_raw_parts(
                        self.as_ptr() as *const u8,
                        mem::size_of_val(self),
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
