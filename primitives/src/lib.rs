#[macro_use]
extern crate beserial_derive;

#[cfg(feature = "lazy_static")]
#[macro_use]
extern crate lazy_static;

#[cfg(feature = "account")]
#[macro_use]
extern crate nimiq_macros as macros;

#[cfg(feature = "nimiq-hash")]
extern crate nimiq_hash as hash;
#[cfg(feature = "nimiq-keys")]
extern crate nimiq_keys as keys;
#[cfg(feature = "nimiq-utils")]
extern crate nimiq_utils as utils;

#[cfg(feature = "account")]
#[macro_use]
extern crate log;

#[cfg(feature = "transaction")]
#[macro_use]
extern crate bitflags;

#[cfg(feature = "coin")]
pub mod coin;
#[cfg(feature = "account")]
pub mod account;
#[cfg(feature = "block")]
pub mod block;
#[cfg(feature = "policy")]
pub mod policy;
#[cfg(feature = "transaction")]
pub mod transaction;
#[cfg(feature = "networks")]
pub mod networks;


use beserial::{Deserialize, Serialize};
use block::{Block, BlockHeader};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainProof {
    #[beserial(len_type(u16))]
    pub prefix: Vec<Block>,
    #[beserial(len_type(u16))]
    pub suffix: Vec<BlockHeader>
}
