#![feature(crate_visibility_modifier)]

#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
extern crate nimiq_primitives as primitives;
extern crate nimiq_hash as hash;
extern crate nimiq_database as database;
extern crate nimiq_keys as keys;
extern crate nimiq_network_primitives as network_primitives;

pub mod tree;
pub mod accounts;
pub mod accounts_proof;

pub use self::accounts::Accounts;
pub use self::accounts_proof::AccountsProof;

use beserial::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountsTreeChunk {}
