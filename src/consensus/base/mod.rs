pub mod account;
pub mod block;
pub mod primitive;
pub mod transaction;

use beserial::{Serialize, Deserialize};

#[derive(Serialize,Deserialize)]
pub struct Subscription { }
