use std::collections::HashMap;

use nimiq_block_albatross::{Block, MacroHeader};
use nimiq_blockchain_albatross::ChainInfo;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy;

#[derive(Debug)]
pub struct ChainStore {
    // A store of chain infos indexed by their block hashes. Contains only headers.
    chain_db: HashMap<Blake2bHash, ChainInfo>,
    // A store of block hashes indexed by their block number.
    height_idx: HashMap<u32, Vec<Blake2bHash>>,
    // A store of election block headers indexed by their epoch number.
    election_db: HashMap<u32, MacroHeader>,
}

impl ChainStore {
    pub fn new() -> Self {
        ChainStore {
            chain_db: HashMap::new(),
            height_idx: HashMap::new(),
            election_db: HashMap::new(),
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

    pub fn put_chain_info(&mut self, mut chain_info: ChainInfo) {
        let hash = chain_info.head.hash();

        // Delete the body and the justification, if they exist.
        if chain_info.head.body().is_some() || chain_info.head.justification().is_some() {
            match &mut chain_info.head {
                Block::Macro(ref mut block) => {
                    block.body = None;
                    block.justification = None
                }
                Block::Micro(ref mut block) => {
                    block.body = None;
                    block.justification = None
                }
            }
        }

        assert!(chain_info.head.body().is_none());
        assert!(chain_info.head.justification().is_none());

        let previous = self.chain_db.insert(hash.clone(), chain_info.clone());

        // If the block was already in the ChainStore then we don't need to modify the height index.
        if previous.is_none() {
            self.height_idx
                .entry(chain_info.head.block_number())
                .or_default()
                .push(hash);
        }
    }

    pub fn get_election(&self, epoch_number: u32) -> Option<&MacroHeader> {
        self.election_db.get(&epoch_number)
    }

    pub fn put_election(&mut self, header: MacroHeader) {
        self.election_db
            .insert(policy::epoch_at(header.block_number), header);
    }

    pub fn clear(&mut self) {
        self.chain_db.clear();
        self.height_idx.clear();
    }
}
