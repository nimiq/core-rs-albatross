use std::{collections::HashMap, ops};

use nimiq_block::{Block, MacroHeader};
use nimiq_blockchain_interface::{BlockchainError, ChainInfo, Direction};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy::Policy;

/// A struct that stores the blocks for the blockchain.
#[derive(Debug, Default)]
pub struct ChainStore {
    // A store of chain infos indexed by their block hashes. Contains only headers.
    chain_db: HashMap<Blake2bHash, ChainInfo>,
    // A store of block hashes indexed by their block number.
    height_idx: HashMap<u32, Vec<Blake2bHash>>,
    // A store of election block headers indexed by their epoch number.
    election_db: HashMap<u32, MacroHeader>,
}

impl ChainStore {
    /// Gets a chain info by its hash. Returns None if the chain info doesn't exist.
    pub fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
    ) -> Result<&ChainInfo, BlockchainError> {
        let chain_info = self
            .chain_db
            .get(hash)
            .ok_or(BlockchainError::BlockNotFound)?;
        if include_body && chain_info.head.body().is_none() {
            return Err(BlockchainError::BlockBodyNotFound);
        }
        Ok(chain_info)
    }

    /// Gets all the stored block hashes for a given block number (you can have several micro blocks
    /// with the same block number because of forks). Returns None if there are no block hashes for
    /// that block number.
    pub fn get_block_hashes(&self, block_number: &u32) -> Option<&[Blake2bHash]> {
        self.height_idx.get(block_number).map(ops::Deref::deref)
    }

    /// Gets a chain info by its block number. Returns None if the chain info doesn't exist.
    /// If there are multiple blocks at that block number, it will return the block that is on the
    /// main chain.
    pub fn get_chain_info_at(
        &self,
        block_height: u32,
        include_body: bool,
    ) -> Result<ChainInfo, BlockchainError> {
        // Get block hashes at the given height.
        let block_hashes = self
            .get_block_hashes(&block_height)
            .ok_or(BlockchainError::BlockNotFound)?;

        // Iterate until we find the main chain block.
        for hash in block_hashes {
            let chain_info = self.get_chain_info(hash, include_body)?;

            // If it's on the main chain we can return from loop
            if chain_info.on_main_chain {
                return Ok(chain_info.clone());
            }
        }

        Err(BlockchainError::BlockNotFound)
    }

    /// Adds a chain info to the ChainStore.
    pub fn put_chain_info(&mut self, mut chain_info: ChainInfo) {
        // Get the block hash.
        let hash = chain_info.head.hash();

        // We only store in the ChainStore:
        // - Micro blocks: Headers and Justifications
        // - Macro blocks: Headers
        match &mut chain_info.head {
            Block::Macro(ref mut block) => {
                block.body = None;
                block.justification = None;
            }
            Block::Micro(ref mut block) => {
                block.body = None;
            }
        }

        // Add the chain info to the chain_db. If there was already a chain info at the same hash, it
        // will return an Option with the previous chain info.
        let previous = self.chain_db.insert(hash.clone(), chain_info.clone());

        // If the block was already in the ChainStore then we don't need to modify the height index.
        // Otherwise, we need to add this block hash at the block height.
        if previous.is_none() {
            self.height_idx
                .entry(chain_info.head.block_number())
                .or_default()
                .push(hash);
        }
    }

    /// Gets an election block header by its epoch number. Returns None if such a election block
    /// doesn't exist.
    pub fn get_election(&self, epoch_number: u32) -> Option<&MacroHeader> {
        self.election_db.get(&epoch_number)
    }

    /// Returns None if given start_block_hash is not a macro block.
    pub fn get_macro_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        direction: Direction,
        election_blocks_only: bool,
        include_body: bool,
    ) -> Result<Vec<Block>, BlockchainError> {
        match direction {
            Direction::Forward => self.get_macro_blocks_forward(
                start_block_hash,
                count,
                election_blocks_only,
                include_body,
            ),
            Direction::Backward => self.get_macro_blocks_backward(
                start_block_hash,
                count,
                election_blocks_only,
                include_body,
            ),
        }
    }

    /// Returns None if given start_block_hash is not a macro block.
    fn get_macro_blocks_backward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        election_blocks_only: bool,
        include_body: bool,
    ) -> Result<Vec<Block>, BlockchainError> {
        let mut blocks = Vec::new();
        let start_block = match self
            .chain_db
            .get(start_block_hash)
            .map(|chain_info| chain_info.head.clone())
        {
            Some(Block::Macro(block)) => block,
            Some(_) => {
                // Expected a macro block and received a micro block
                return Err(BlockchainError::BlockIsNotMacro);
            }
            None => return Err(BlockchainError::BlockNotFound),
        };

        let mut hash = if election_blocks_only {
            start_block.header.parent_election_hash
        } else {
            start_block.header.parent_hash
        };
        while (blocks.len() as u32) < count {
            let block_opt = self
                .chain_db
                .get(&hash)
                .map(|chain_info| chain_info.head.clone());
            if let Some(Block::Macro(block)) = block_opt {
                hash = if election_blocks_only {
                    block.header.parent_election_hash.clone()
                } else {
                    block.header.parent_hash.clone()
                };
                if include_body && block.body.is_none() {
                    return Err(BlockchainError::BlockBodyNotFound);
                }
                blocks.push(Block::Macro(block));
            } else {
                break;
            }
        }

        Ok(blocks)
    }

    /// Returns None if given start_block_hash is not a macro block.
    fn get_macro_blocks_forward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        election_blocks_only: bool,
        include_body: bool,
    ) -> Result<Vec<Block>, BlockchainError> {
        let mut blocks = Vec::new();
        let block = match self
            .chain_db
            .get(start_block_hash)
            .map(|chain_info| chain_info.head.clone())
        {
            Some(Block::Macro(block)) => block,
            Some(_) => {
                // Expected a macro block and received a micro block
                return Err(BlockchainError::BlockIsNotMacro);
            }
            None => return Err(BlockchainError::BlockNotFound),
        };

        let mut next_macro_block = if election_blocks_only {
            Policy::election_block_after(block.header.block_number)
        } else {
            Policy::macro_block_after(block.header.block_number)
        };
        while (blocks.len() as u32) < count {
            let block_result = self
                .get_chain_info_at(next_macro_block, include_body)
                .map(|chain_info| chain_info.head);
            match block_result {
                Ok(Block::Macro(block)) => {
                    next_macro_block = if election_blocks_only {
                        Policy::election_block_after(block.header.block_number)
                    } else {
                        Policy::macro_block_after(block.header.block_number)
                    };
                    blocks.push(Block::Macro(block));
                }
                Ok(_) => {
                    // Expected a macro block and received a micro block
                    return Err(BlockchainError::InconsistentState);
                }
                Err(BlockchainError::BlockBodyNotFound) => {
                    return Err(BlockchainError::BlockBodyNotFound)
                }
                Err(BlockchainError::BlockNotFound) => break,
                Err(e) => return Err(e),
            }
        }

        Ok(blocks)
    }

    pub fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        direction: Direction,
        include_body: bool,
    ) -> Result<Vec<Block>, BlockchainError> {
        match direction {
            Direction::Forward => self.get_blocks_forward(start_block_hash, count, include_body),
            Direction::Backward => self.get_blocks_backward(start_block_hash, count, include_body),
        }
    }

    pub fn get_blocks_at(&self, block_height: u32) -> Vec<Block> {
        // Look for the block hashes at the indicated height
        let mut blocks = Vec::new();
        let block_hashes = self.height_idx.get(&block_height);
        if let Some(block_hashes) = block_hashes {
            // We found some hashes for the indicated height. Look for the blocks in the chain DB.
            for hash in block_hashes {
                if let Some(block) = self
                    .chain_db
                    .get(hash)
                    .map(|chain_info| chain_info.head.clone())
                {
                    blocks.push(block)
                }
            }
        }

        blocks
    }

    fn get_blocks_backward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
    ) -> Result<Vec<Block>, BlockchainError> {
        let mut blocks = Vec::new();
        let start_block = self
            .chain_db
            .get(start_block_hash)
            .map(|chain_info| chain_info.head.clone())
            .ok_or(BlockchainError::BlockNotFound)?;

        let mut hash = start_block.parent_hash().clone();
        while (blocks.len() as u32) < count {
            if let Some(block) = self
                .chain_db
                .get(&hash)
                .map(|chain_info| chain_info.head.clone())
            {
                if include_body && block.body().is_none() {
                    return Err(BlockchainError::BlockBodyNotFound);
                }
                hash = block.parent_hash().clone();
                blocks.push(block);
            } else {
                break;
            }
        }

        Ok(blocks)
    }

    fn get_blocks_forward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
    ) -> Result<Vec<Block>, BlockchainError> {
        let mut blocks = Vec::new();
        let mut chain_info = self
            .chain_db
            .get(start_block_hash)
            .ok_or(BlockchainError::BlockNotFound)?;

        while (blocks.len() as u32) < count {
            if let Some(ref successor) = chain_info.main_chain_successor {
                let chain_info_opt = self.chain_db.get(successor);
                if chain_info_opt.is_none() {
                    break;
                }

                chain_info = chain_info_opt.unwrap();
                let block = chain_info.head.clone();
                if include_body && block.body().is_none() {
                    return Err(BlockchainError::BlockBodyNotFound);
                }
                blocks.push(block);
            } else {
                break;
            }
        }

        Ok(blocks)
    }

    /// Adds an election block header to the ChainStore.
    pub fn put_election(&mut self, header: MacroHeader) {
        self.election_db
            .insert(Policy::epoch_at(header.block_number), header);
    }

    /// Clears the ChainStore of all blocks (except the election blocks). This can be used at the
    /// end of each batch, so that we don't keep unnecessary micro blocks.
    pub fn clear(&mut self) {
        self.chain_db.clear();
        self.height_idx.clear();
    }
}

#[cfg(test)]
mod tests {
    use nimiq_block::{MicroBlock, MicroHeader, MicroJustification};
    use nimiq_hash::Blake2sHash;
    use nimiq_primitives::networks::NetworkId;
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng::test_rng;
    use rand::{Rng, RngCore};

    use super::*;

    #[test]
    fn put_and_get_works() {
        // Create blocks.
        let mut data = [0u8; 32];
        let mut rng = test_rng(false);
        rng.fill_bytes(&mut data);
        let hash_1 = Blake2bHash::from(data);

        let block_1 = Block::Micro(MicroBlock {
            header: MicroHeader {
                network: NetworkId::UnitAlbatross,
                version: rng.gen(),
                block_number: 0,
                timestamp: rng.gen(),
                parent_hash: hash_1.clone(),
                seed: Default::default(),
                extra_data: vec![],
                state_root: hash_1.clone(),
                body_root: Blake2sHash::default(),
                diff_root: Blake2bHash::default(),
                history_root: hash_1,
            },
            justification: Some(MicroJustification::Micro(Default::default())),
            body: None,
        });

        let mut data = [0u8; 32];
        rng.fill_bytes(&mut data);
        let hash_2 = Blake2bHash::from(data);

        let block_2 = Block::Micro(MicroBlock {
            header: MicroHeader {
                network: NetworkId::UnitAlbatross,
                version: rng.gen(),
                block_number: 0,
                timestamp: rng.gen(),
                parent_hash: hash_2.clone(),
                seed: Default::default(),
                extra_data: vec![],
                state_root: hash_2.clone(),
                body_root: Blake2sHash::default(),
                diff_root: Blake2bHash::default(),
                history_root: hash_2,
            },
            justification: Some(MicroJustification::Micro(Default::default())),
            body: None,
        });

        // Create chain store.
        let mut store = ChainStore::default();

        // First case.
        store.put_chain_info(ChainInfo::new(block_1.clone(), true));
        store.put_chain_info(ChainInfo::new(block_2.clone(), false));

        match store.get_chain_info_at(0, false) {
            Err(e) => {
                assert!(true, "Error getting chain info: {e}")
            }
            Ok(info) => {
                assert!(info.on_main_chain);
                assert!(info.head.body().is_none());
                assert_eq!(info.head.hash(), block_1.hash());
            }
        }

        store.clear();

        // Second case.
        store.put_chain_info(ChainInfo::new(block_1.clone(), true));
        store.put_chain_info(ChainInfo::new(block_2.clone(), true));

        match store.get_chain_info_at(0, false) {
            Err(e) => {
                assert!(true, "Error getting chain info: {e}")
            }
            Ok(info) => {
                assert!(info.on_main_chain);
                assert!(info.head.body().is_none());
                assert_eq!(info.head.hash(), block_1.hash());
            }
        }

        // Third case.
        store.put_chain_info(ChainInfo::new(block_1, false));
        store.put_chain_info(ChainInfo::new(block_2, false));

        match store.get_chain_info_at(0, false) {
            Err(_) => {}
            Ok(_) => {
                assert!(true, "Expected error but found Ok instead")
            }
        }
    }
}
