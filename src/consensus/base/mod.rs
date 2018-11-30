use beserial::{Deserialize, Serialize};

pub mod account;
pub mod block;
pub mod blockchain;
pub mod mempool;
pub mod primitive;
pub mod transaction;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subscription {

}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum SubscriptionType {
    None = 0,
    Any = 1,
    Addresses = 2,
    MinFee = 3
}
