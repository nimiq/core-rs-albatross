use std::borrow::Cow;
use std::io;

use beserial::{Deserialize, Serialize};
use nimiq_account::{AccountsTreeLeave, Receipts};
use nimiq_tree::accounts_tree_node::AccountsTreeNode;
use nimiq_tree::address_nibbles::AddressNibbles;

use crate::{AsDatabaseBytes, FromDatabaseValue, IntoDatabaseValue};

impl AsDatabaseBytes for AddressNibbles {
    fn as_database_bytes(&self) -> Cow<[u8]> {
        // TODO: Improve AddressNibbles, so that no serialization is needed.
        let v = Serialize::serialize_to_vec(&self);
        Cow::Owned(v)
    }
}

impl<A: AccountsTreeLeave> IntoDatabaseValue for AccountsTreeNode<A> {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl<A: AccountsTreeLeave> FromDatabaseValue for AccountsTreeNode<A> {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}

impl IntoDatabaseValue for Receipts {
    fn database_byte_size(&self) -> usize {
        self.serialized_size()
    }

    fn copy_into_database(&self, mut bytes: &mut [u8]) {
        Serialize::serialize(&self, &mut bytes).unwrap();
    }
}

impl FromDatabaseValue for Receipts {
    fn copy_from_database(bytes: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        let mut cursor = io::Cursor::new(bytes);
        Ok(Deserialize::deserialize(&mut cursor)?)
    }
}
