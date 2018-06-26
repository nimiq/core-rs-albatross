use beserial::{Serialize, Deserialize};
use beserial_derive;

#[derive(Clone,Copy,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
#[repr(u8)]
pub enum AccountType {
    Basic = 0,
}
