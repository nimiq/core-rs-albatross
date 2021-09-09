use std::convert::TryFrom;

use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_trie::key_nibbles::KeyNibbles;

use crate::Account;

/// A small wrapper over a list of accounts with keys. This is only used to have method
/// of serializing and deserializing the genesis accounts.
pub struct AccountsList(pub Vec<(KeyNibbles, Account)>);

impl Deserialize for AccountsList {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let count: u16 = Deserialize::deserialize(reader)?;
        let mut accounts: Vec<(KeyNibbles, Account)> = Vec::new();
        for _ in 0..count {
            accounts.push((
                Deserialize::deserialize(reader)?,
                Deserialize::deserialize(reader)?,
            ));
        }
        Ok(Self(accounts))
    }
}

impl Serialize for AccountsList {
    fn serialize<W: WriteBytesExt>(&self, writer: &mut W) -> Result<usize, SerializingError> {
        let mut size = 0;
        let count: u16 = u16::try_from(self.0.len()).map_err(|_| SerializingError::Overflow)?;
        size += count.serialize(writer)?;
        for (key, account) in self.0.iter() {
            size += key.serialize(writer)?;
            size += account.serialize(writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 2; // count as u16
        for (key, account) in self.0.iter() {
            size += key.serialized_size();
            size += account.serialized_size();
        }
        size
    }
}
