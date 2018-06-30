use beserial::{Deserialize, Serialize};
use consensus::base::primitive::Address;
use consensus::base::primitive::hash::{Hash, SerializeContent};
use std::io;

pub mod tree;

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum AccountType {
    Basic = 0,
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct Account {}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct PrunedAccount {
    address: Address,
    account: Account,
}

impl SerializeContent for PrunedAccount {
    fn serialize_content<W: io::Write>(&self, writer: &mut W) -> io::Result<usize> { self.serialize(writer) }
}

impl Hash for PrunedAccount {}
