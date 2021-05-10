use std::io;

use beserial::{Deserialize, Serialize};
use nimiq_block::{Block, MicroBlock};

use crate::{FromDatabaseValue, IntoDatabaseValue};

impl IntoDatabaseValue for Block {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for Block {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}

impl IntoDatabaseValue for MicroBlock {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for MicroBlock {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
