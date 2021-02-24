use std::collections::HashMap;

use nimiq_blockchain_albatross::ChainInfo;
use nimiq_hash::Blake2bHash;

#[derive(Debug)]
pub struct ChainStore {
    // A store of chain infos indexed by their block hashes.
    chain_db: HashMap<Blake2bHash, ChainInfo>,
    // A store of block hashes indexed by their block number.
    height_idx: HashMap<u32, Vec<Blake2bHash>>,
}
impl ChainStore {
    pub fn new() -> Self {
        ChainStore {
            chain_db: HashMap::new(),
            height_idx: HashMap::new(),
        }
    }

    pub fn get_chain_info(&self, hash: &Blake2bHash) -> Option<&ChainInfo> {
        self.chain_db.get(hash)
    }

    pub fn get_block_hashes(&self, block_number: &u32) -> Option<&Vec<Blake2bHash>> {
        self.height_idx.get(block_number)
    }

    pub fn get_chain_info_at(&self, block_height: u32) -> Option<ChainInfo> {
        // Get block hashes at the given height.
        let block_hashes = self.get_block_hashes(&block_height)?;

        // Iterate until we find the main chain block.
        for hash in block_hashes {
            let chain_info = self.get_chain_info(hash)?;

            // If it's on the main chain we can return from loop
            if chain_info.on_main_chain {
                return Some(chain_info.clone());
            }
        }

        unreachable!()
    }

    pub fn put_chain_info(&mut self, chain_info: ChainInfo) {
        let hash = chain_info.head.hash();

        let previous = self.chain_db.insert(hash.clone(), chain_info.clone());

        // If the block was already in the ChainStore then we don't need to modify the height index.
        if previous.is_none() {
            self.height_idx
                .entry(chain_info.head.block_number())
                .or_default()
                .push(hash);
        }
    }

    pub fn clear(&mut self) {
        self.chain_db.clear();
        self.height_idx.clear();
    }
}
