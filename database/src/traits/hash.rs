extern crate hash;

use std::borrow::Cow;
use std::io;

use hash::Blake2bHash;

use crate::{AsDatabaseBytes, FromDatabaseValue};

impl AsDatabaseBytes for Blake2bHash {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        return Cow::Borrowed(self.as_bytes());
    }
}

impl FromDatabaseValue for Blake2bHash {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        return Ok(bytes.into());
    }
}