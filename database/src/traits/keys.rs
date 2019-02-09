extern crate nimiq_keys as keys;

use std::borrow::Cow;
use std::io;

use keys::Address;

use crate::{AsDatabaseBytes, FromDatabaseValue};

impl AsDatabaseBytes for Address {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        return Cow::Borrowed(self.as_bytes());
    }
}

impl FromDatabaseValue for Address {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        return Ok(bytes.into());
    }
}