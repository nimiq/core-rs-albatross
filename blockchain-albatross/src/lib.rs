#[macro_use]
extern crate beserial_derive;
#[macro_use]
extern crate log;

extern crate nimiq_accounts as accounts;
extern crate nimiq_bls as bls;
extern crate nimiq_primitives as primitives;
extern crate nimiq_hash as hash;
extern crate nimiq_database as database;
extern crate nimiq_network_primitives as network_primitives;
extern crate nimiq_utils as utils;
extern crate nimiq_keys as keys;
extern crate nimiq_account as account;
extern crate nimiq_block_albatross as block;
extern crate nimiq_transaction as transaction;
extern crate nimiq_tree_primitives as tree_primitives;

pub mod blockchain;


use primitives::networks::NetworkId;
use hash::Blake2bHash;
use beserial::{Serialize, Deserialize};
use blockchain::Direction;

pub trait AbstractBlockchain {
    type Block: Serialize + Deserialize;
    type VerifyResult;


    /// Returns the network ID
    fn network_id(&self) -> NetworkId;


    /// Returns the current head block
    fn head_block(&self) -> Self::Block;

    /// Returns the current head hash of the active chain
    fn head_hash(&self) -> Blake2bHash;

    /// Returns the height of the current head
    fn head_height(&self) -> u32;


    /// Get block by hash
    fn get_block(&self, hash: &Blake2bHash) -> Option<Self::Block>;

    /// Get block by block number
    fn get_block_at(&self, height: u32) -> Option<Self::Block>;

    /// Get block locators
    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash>;

    /// Get `count` blocks starting from `start_block_hash` into `direction` and optionally include
    /// block bodies.
    fn get_blocks(&self, start_block_hash: &Blake2bHash, count: u32, include_body: bool, direction: Direction) -> Vec<Self::Block>;


    /// Verify a block
    fn verify(&self, block: &Self::Block) -> Self::VerifyResult;

    /// Push a block to the block chain
    fn push(&self, block: Self::Block);


    /// Check if a block with the given hash is included in the blockchain
    /// `include_forks` will also check for micro-block forks in the current epoch
    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool;
}
