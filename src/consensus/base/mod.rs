use beserial::{Deserialize, Serialize};

pub mod account;
pub mod block;
pub mod blockchain;
pub mod mempool;
pub mod primitive;
pub mod transaction;

#[derive(Clone, Serialize, Deserialize)]
pub struct Subscription {
}

pub enum SubscriptionType {
    None = 0,
    Any = 1,
    Addresses = 2,
    MinFee = 3
}
