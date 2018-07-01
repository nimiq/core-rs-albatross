use beserial::{Serialize, Deserialize};

#[derive(Clone,Copy,PartialEq,PartialOrd,Eq,Ord,Debug,Serialize,Deserialize)]
pub struct VestingContract {
    pub balance: u64
}
