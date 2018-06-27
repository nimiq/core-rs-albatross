use beserial::{Serialize, Deserialize};
use beserial_derive;
use consensus::base::primitive::Address;

#[derive(Clone,Copy,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
#[repr(u8)]
pub enum AccountType {
    Basic = 0,
}

#[derive(Clone,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
pub struct Account {}

#[derive(Clone,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
pub struct PrunedAccount {
    address: Address,
    account: Account
}
