use std::borrow::Cow;
use std::io;

use beserial::{Deserialize, Serialize};
use nimiq_tree_primitives::accounts_tree_node::AccountsTreeNode;
use nimiq_tree_primitives::address_nibbles::AddressNibbles;

use crate::{AsDatabaseBytes, FromDatabaseValue, IntoDatabaseValue};

impl AsDatabaseBytes for AddressNibbles {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        // TODO: Improve AddressNibbles, so that no serialization is needed.
        let v = Serialize::serialize_to_vec(&self);
        Cow::Owned(v)
    }
}

impl IntoDatabaseValue for AccountsTreeNode {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for AccountsTreeNode {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self> where Self: Sized {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
