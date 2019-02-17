use std::borrow::Cow;
use std::io;

use nimiq_keys::Address;

use crate::{AsDatabaseBytes, FromDatabaseValue};

impl AsDatabaseBytes for Address {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        Cow::Borrowed(self.as_bytes())
    }
}

impl FromDatabaseValue for Address {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        Ok(bytes.into())
    }
}