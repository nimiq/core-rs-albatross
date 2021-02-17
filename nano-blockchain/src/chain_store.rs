extern crate nimiq_primitives as primitives;

use crate::chain_info::ChainInfo;
use nimiq_block_albatross::Block;
use nimiq_hash::Blake2bHash;
use std::collections::HashMap;

#[derive(Debug)]
pub struct ChainStore {
    // A store of chain infos indexed by their block hashes.
    chain_db: HashMap<Blake2bHash, ChainInfo>,
    // A database of blocks indexed by their block hashes.
    block_db: HashMap<Blake2bHash, Block>,
    // A database of block hashes indexed by their block number.
    height_idx: HashMap<u32, Vec<Blake2bHash>>,
}
impl ChainStore {
    pub fn new() -> Self {
        ChainStore {
            chain_db: HashMap::new(),
            block_db: HashMap::new(),
            height_idx: HashMap::new(),
        }
    }

    pub fn get_chain_info(&self, hash: &Blake2bHash) -> Option<&ChainInfo> {
        self.chain_db.get(hash)
    }

    pub fn get_block(&self, hash: &Blake2bHash) -> Option<&Block> {
        self.block_db.get(hash)
    }

    pub fn get_block_hashes(&self, block_number: &u32) -> Option<&Vec<Blake2bHash>> {
        self.height_idx.get(block_number)
    }

    pub fn put_chain_info(&mut self, hash: Blake2bHash, chain_info: ChainInfo) {
        self.chain_db.insert(hash, chain_info);
    }

    pub fn put_block(&mut self, hash: Blake2bHash, block: Block) {
        self.block_db.insert(hash, block);
    }

    pub fn put_block_hash(&mut self, block_number: u32, hash: Blake2bHash) {
        self.height_idx.entry(block_number).or_default().push(hash)
    }
}
