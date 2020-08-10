#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;
extern crate nimiq_account as account;
extern crate nimiq_accounts as accounts;
extern crate nimiq_block_albatross as block;
extern crate nimiq_blockchain_base as blockchain_base;
extern crate nimiq_bls as bls;
extern crate nimiq_collections as collections;
extern crate nimiq_database as database;
extern crate nimiq_genesis as genesis;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_tree_primitives as tree_primitives;
extern crate nimiq_utils as utils;
extern crate nimiq_vrf as vrf;

use block::{Block, BlockError, ForkProof};
pub use blockchain::Blockchain;

pub mod blockchain;
pub mod blockchain_state;
pub mod chain_info;
pub mod chain_store;
pub mod reward;
pub mod slots;
pub mod transaction_cache;

pub type PushResult = blockchain_base::PushResult;
pub type PushError = blockchain_base::PushError<BlockError>;
pub type BlockchainEvent = blockchain_base::BlockchainEvent<Block>;

#[derive(Debug, Eq, PartialEq)]
enum ChainOrdering {
    Extend,
    Better,
    Inferior,
    Unknown,
}

pub enum ForkEvent {
    Detected(ForkProof),
}
