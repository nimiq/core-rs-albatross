use beserial::{Serialize, Deserialize};

#[derive(Clone,Copy,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
pub struct VestingContract {
    balance: u64
}
