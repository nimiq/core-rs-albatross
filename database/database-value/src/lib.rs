use std::{borrow::Cow, ffi::CStr, mem, slice};

pub trait IntoDatabaseValue {
    fn database_byte_size(&self) -> usize;
    fn copy_into_database(&self, bytes: &mut [u8]);
}

pub trait FromDatabaseBytes {
    /// Reads the value from the database key bytes.
    /// In the default implementation, this will also be used for the value bytes.
    fn from_key_bytes(bytes: &[u8]) -> Self
    where
        Self: Sized;

    /// Reads the value from the database bytes (in value encoding).
    /// This is only necessary if the value needs to be sorted differently in a dup table.
    /// DUP values use a lexicographic sort order.
    fn from_value_bytes(bytes: &[u8]) -> Self
    where
        Self: Sized,
    {
        Self::from_key_bytes(bytes)
    }
}

pub trait AsDatabaseBytes {
    /// Returns the byte representation to be used in key encoding.
    /// In the default implementation, this will also be used for the value bytes.
    fn as_key_bytes(&self) -> Cow<[u8]>;

    /// Returns the byte representation to be used in value encoding.
    /// This is used if the value needs to be sorted differently in a dup table.
    /// DUP values use a lexicographic sort order.
    fn as_value_bytes(&self) -> Cow<[u8]> {
        self.as_key_bytes()
    }

    /// Determines whether the values are of fixed size.
    /// This will set the `DUP_FIXED_SIZE_VALUES` for dup tables flag.
    const FIXED_SIZE: Option<usize> = None;
}

impl AsDatabaseBytes for () {
    fn as_key_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(&[])
    }

    const FIXED_SIZE: Option<usize> = Some(0);
}

impl FromDatabaseBytes for () {
    fn from_key_bytes(bytes: &[u8]) -> Self
    where
        Self: Sized,
    {
        assert!(bytes.is_empty());
    }
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

impl FromDatabaseBytes for String {
    fn from_key_bytes(bytes: &[u8]) -> Self
    where
        Self: Sized,
    {
        String::from_utf8(bytes.to_vec()).unwrap()
    }
}

impl FromDatabaseBytes for Vec<u8> {
    fn from_key_bytes(bytes: &[u8]) -> Self
    where
        Self: Sized,
    {
        bytes.to_vec()
    }
}

impl AsDatabaseBytes for Vec<u8> {
    fn as_key_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(&self[..])
    }
}

impl AsDatabaseBytes for String {
    fn as_key_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(self.as_bytes())
    }
}

impl AsDatabaseBytes for str {
    fn as_key_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(self.as_bytes())
    }
}

impl AsDatabaseBytes for CStr {
    fn as_key_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(self.to_bytes())
    }
}

macro_rules! impl_num_traits {
    ($typ:ident) => {
        impl FromDatabaseBytes for $typ {
            fn from_key_bytes(bytes: &[u8]) -> Self
            where
                Self: Sized,
            {
                $typ::from_ne_bytes(bytes.try_into().expect("mismatch size"))
            }

            fn from_value_bytes(bytes: &[u8]) -> Self
            where
                Self: Sized,
            {
                // It is important to use to_be_bytes here for lexical ordering.
                $typ::from_be_bytes(bytes.try_into().expect("mismatch size"))
            }
        }
        impl AsDatabaseBytes for $typ {
            fn as_key_bytes(&self) -> Cow<[u8]> {
                unsafe {
                    #[allow(clippy::size_of_in_element_count)]
                    Cow::Borrowed(slice::from_raw_parts(
                        self as *const $typ as *const u8,
                        mem::size_of::<$typ>(),
                    ))
                }
            }

            fn as_value_bytes(&self) -> Cow<[u8]> {
                // It is important to use to_be_bytes here for lexical ordering.
                Cow::Owned(self.to_be_bytes().to_vec())
            }

            const FIXED_SIZE: Option<usize> = Some($typ::BITS as usize / 8);
        }
    };
}

impl_num_traits!(u16);
impl_num_traits!(u32);
impl_num_traits!(u64);
