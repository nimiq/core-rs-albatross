use database::{Database, DatabaseFlags, Environment, ReadTransaction, Transaction, WriteTransaction};
use hash::Blake2bHash;
use block::Block;

use crate::chain_info::ChainInfo;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Direction {
    Forward,
    Backward,
}

#[derive(Debug)]
pub struct ChainStore<'env> {
    env: &'env Environment,
    chain_db: Database<'env>,
    block_db: Database<'env>,
    height_idx: Database<'env>
}

impl<'env> ChainStore<'env> {
    const CHAIN_DB_NAME: &'static str = "ChainData";
    const BLOCK_DB_NAME: &'static str = "Block";
    const HEIGHT_IDX_NAME: &'static str = "HeightIdx";
    const HEAD_KEY: &'static str = "head";

    pub fn new(env: &'env Environment) -> Self {
        let chain_db = env.open_database(Self::CHAIN_DB_NAME.to_string());
        let block_db = env.open_database(Self::BLOCK_DB_NAME.to_string());
        let height_idx = env.open_database_with_flags(Self::HEIGHT_IDX_NAME.to_string(),
            DatabaseFlags::DUPLICATE_KEYS | DatabaseFlags::DUP_FIXED_SIZE_VALUES);
        ChainStore { env, chain_db, block_db, height_idx }
    }

    pub fn get_head(&self, txn_option: Option<&Transaction>) -> Option<Blake2bHash> {
        match txn_option {
            Some(txn) => txn.get(&self.chain_db, ChainStore::HEAD_KEY),
            None => ReadTransaction::new(self.env).get(&self.chain_db, ChainStore::HEAD_KEY)
        }
    }

    pub fn set_head(&self, txn: &mut WriteTransaction, hash: &Blake2bHash) {
        txn.put(&self.chain_db, ChainStore::HEAD_KEY, hash);
    }

    pub fn get_chain_info(&self, hash: &Blake2bHash, include_body: bool, txn_option: Option<&Transaction>) -> Option<ChainInfo> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        let mut chain_info: ChainInfo = match txn.get(&self.chain_db, hash) {
            Some(data) => data,
            None => return None
        };

        if include_body {
            if let Some(block) = txn.get(&self.block_db, hash) {
                chain_info.head = block;
            } else {
                warn!("Block body requested but not present");
            }
        }

        Some(chain_info)
    }

    pub fn put_chain_info(&self, txn: &mut WriteTransaction, hash: &Blake2bHash, chain_info: &ChainInfo, include_body: bool) {
        // Store chain data. Block body will not be persisted.
        txn.put_reserve(&self.chain_db, hash, chain_info);

        // Store body if requested.
        if include_body && chain_info.head.body.is_some() {
            txn.put_reserve(&self.block_db, hash, &chain_info.head);
        }

        // Add to height index.
        let height = chain_info.head.header.height;
        txn.put(&self.height_idx, &height, hash);
    }

    pub fn remove_chain_info(&self, txn: &mut WriteTransaction, hash: &Blake2bHash, height: u32) {
        txn.remove(&self.chain_db, hash);
        txn.remove(&self.block_db, hash);
        txn.remove_item(&self.height_idx, &height, hash);
    }

    pub fn get_chain_info_at(&self, block_height: u32, include_body: bool, txn_option: Option<&Transaction>) -> Option<ChainInfo> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        // Seek to the first block at the given height.
        let mut cursor = txn.cursor(&self.height_idx);
        let mut block_hash = match cursor.seek_key::<u32, Blake2bHash>(&block_height) {
            Some(hash) => hash,
            None => return None
        };

        // Iterate until we find the main chain block.
        let mut chain_info: ChainInfo;
        while {
            // Loop condition
            chain_info = txn
                .get(&self.chain_db, &block_hash)
                .expect("Corrupted store: ChainInfo referenced from index not found");
            !chain_info.on_main_chain
        } {
            // Loop Body
            block_hash = match cursor.next_duplicate::<u32, Blake2bHash>() {
                Some((_, hash)) => hash,
                None => return None
            };
        }

        if include_body {
            if let Some(block) = txn.get(&self.block_db, &block_hash) {
                chain_info.head = block;
            } else {
                warn!("Block body requested but not present");
            }
        }

        Some(chain_info)
    }

    pub fn get_block(&self, hash: &Blake2bHash, include_body: bool, txn_option: Option<&Transaction>) -> Option<Block> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        if include_body {
            txn.get(&self.block_db, hash)
        } else {
            txn.get(&self.chain_db, hash).map(|chain_info: ChainInfo| chain_info.head)
        }
    }

    pub fn get_block_at(&self, block_height: u32) -> Option<Block> {
        self.get_chain_info_at(block_height, true, None)
            .map(|chain_info| chain_info.head)
    }

    pub fn get_blocks_backward(&self, start_block_hash: &Blake2bHash, count: u32, include_body: bool, txn_option: Option<&Transaction>) -> Vec<Block> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        let mut blocks= Vec::new();
        let start_block = match self.get_block(start_block_hash, false, Some(&txn)) {
            Some(block) => block,
            None => return blocks
        };

        let mut hash = start_block.header.prev_hash;
        while (blocks.len() as u32) < count {
            let block_opt = self.get_block(&hash, include_body, Some(&txn));
            if block_opt.is_none() {
                break;
            }

            let block = block_opt.unwrap();
            hash = block.header.prev_hash.clone();
            blocks.push(block);
        }

        blocks
    }

    pub fn get_blocks_forward(&self, start_block_hash: &Blake2bHash, count: u32, include_body: bool, txn_option: Option<&Transaction>) -> Vec<Block> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        let mut blocks= Vec::new();
        let mut chain_info = match self.get_chain_info(start_block_hash, false, Some(&txn)) {
            Some(chain_info) => chain_info,
            None => return blocks
        };

        while (blocks.len() as u32) < count {
            if let Some(ref successor) = chain_info.main_chain_successor {
                let chain_info_opt = self.get_chain_info(successor, include_body, Some(&txn));
                if chain_info_opt.is_none() {
                    break;
                }

                chain_info = chain_info_opt.unwrap();
                blocks.push(chain_info.head);
            } else {
                break;
            }
        }

        blocks
    }

    pub fn get_blocks(&self, start_block_hash: &Blake2bHash, count: u32, include_body: bool, direction: Direction, txn_option: Option<&Transaction>) -> Vec<Block> {
        match direction {
            Direction::Forward => self.get_blocks_forward(start_block_hash, count, include_body, txn_option),
            Direction::Backward => self.get_blocks_backward(start_block_hash, count, include_body, txn_option),
        }
    }
}
