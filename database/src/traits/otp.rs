use std::io;

use beserial::{Deserialize, Serialize};
use nimiq_utils::otp::Locked;

use crate::{FromDatabaseValue, IntoDatabaseValue};

impl<T: Default + Deserialize + Serialize> IntoDatabaseValue for Locked<T> {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl<T: Default + Deserialize + Serialize> FromDatabaseValue for Locked<T> {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
