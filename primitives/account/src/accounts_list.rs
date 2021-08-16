use std::convert::TryFrom;

use crate::Account;
use beserial::{Deserialize, ReadBytesExt, Serialize, SerializingError, WriteBytesExt};
use nimiq_keys::Address;

/// A small wrapper over a list of accounts with addresses. This is only used to have method
/// of serializing and deserializing the genesis accounts.
pub struct AccountsList(pub Vec<(Address, Account)>);

impl Deserialize for AccountsList {
    fn deserialize<R: ReadBytesExt>(reader: &mut R) -> Result<Self, SerializingError> {
        let count: u16 = Deserialize::deserialize(reader)?;
        let mut accounts: Vec<(Address, Account)> = Vec::new();
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
        for (address, account) in self.0.iter() {
            size += address.serialize(writer)?;
            size += account.serialize(writer)?;
        }
        Ok(size)
    }

    fn serialized_size(&self) -> usize {
        let mut size = 2; // count as u16
        for (address, account) in self.0.iter() {
            size += address.serialized_size();
            size += account.serialized_size();
        }
        size
    }
}
