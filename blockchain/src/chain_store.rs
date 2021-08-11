use nimiq_account::Receipts;
use nimiq_block::Block;
use nimiq_database::cursor::{ReadCursor, WriteCursor};
use nimiq_database::{
    Database, DatabaseFlags, Environment, ReadTransaction, Transaction, WriteTransaction,
};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy;

use crate::chain_info::ChainInfo;
use crate::Direction;

#[derive(Debug)]
pub struct ChainStore {
    env: Environment,
    // A database of chain infos (it excludes the block body) indexed by their block hashes.
    chain_db: Database,
    // A database of block bodies indexed by their block hashes.
    block_db: Database,
    // A database of block hashes indexed by their block number.
    height_idx: Database,
    // A database of the transaction receipts for a block, by their corresponding block hashes.
    receipt_db: Database,
}

impl ChainStore {
    const CHAIN_DB_NAME: &'static str = "ChainData";
    const BLOCK_DB_NAME: &'static str = "Block";
    const HEIGHT_IDX_NAME: &'static str = "HeightIndex";
    const RECEIPT_DB_NAME: &'static str = "Receipts";

    const HEAD_KEY: &'static str = "head";

    pub fn new(env: Environment) -> Self {
        let chain_db = env.open_database(Self::CHAIN_DB_NAME.to_string());
        let block_db = env.open_database(Self::BLOCK_DB_NAME.to_string());
        let height_idx = env.open_database_with_flags(
            Self::HEIGHT_IDX_NAME.to_string(),
            DatabaseFlags::DUPLICATE_KEYS | DatabaseFlags::DUP_FIXED_SIZE_VALUES,
        );
        let receipt_db = env
            .open_database_with_flags(Self::RECEIPT_DB_NAME.to_string(), DatabaseFlags::UINT_KEYS);
        ChainStore {
            env,
            chain_db,
            block_db,
            height_idx,
            receipt_db,
        }
    }

    pub fn get_head(&self, txn_option: Option<&Transaction>) -> Option<Blake2bHash> {
        match txn_option {
            Some(txn) => txn.get(&self.chain_db, ChainStore::HEAD_KEY),
            None => ReadTransaction::new(&self.env).get(&self.chain_db, ChainStore::HEAD_KEY),
        }
    }

    pub fn set_head(&self, txn: &mut WriteTransaction, hash: &Blake2bHash) {
        txn.put(&self.chain_db, ChainStore::HEAD_KEY, hash);
    }

    pub fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<ChainInfo> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        let mut chain_info: ChainInfo = match txn.get(&self.chain_db, hash) {
            Some(data) => data,
            None => return None,
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

    pub fn put_chain_info(
        &self,
        txn: &mut WriteTransaction,
        hash: &Blake2bHash,
        chain_info: &ChainInfo,
        include_body: bool,
    ) {
        // Store chain data. Block body will not be persisted because the serialization of ChainInfo
        // ignores the block body.
        txn.put_reserve(&self.chain_db, hash, chain_info);

        // Store body if requested.
        if include_body {
            txn.put_reserve(&self.block_db, hash, &chain_info.head);
        }

        // Add to height index.
        let height = chain_info.head.block_number();
        txn.put(&self.height_idx, &height, hash);
    }

    pub fn remove_chain_info(&self, txn: &mut WriteTransaction, hash: &Blake2bHash, height: u32) {
        txn.remove(&self.chain_db, hash);
        txn.remove(&self.block_db, hash);
        txn.remove_item(&self.height_idx, &height, hash);
    }

    pub fn get_chain_info_at(
        &self,
        block_height: u32,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<ChainInfo> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        // Seek to the first block at the given height.
        let mut cursor = txn.cursor(&self.height_idx);
        let mut block_hash = cursor.seek_key::<u32, Blake2bHash>(&block_height)?;

        // Iterate until we find the main chain block.
        let mut chain_info = loop {
            let chain_info: ChainInfo = txn
                .get(&self.chain_db, &block_hash)
                .expect("Corrupted store: ChainInfo referenced from index not found");

            // If it's on the main chain we can return from loop
            if chain_info.on_main_chain {
                break chain_info;
            }

            // Get next block hash
            block_hash = match cursor.next_duplicate::<u32, Blake2bHash>() {
                Some((_, hash)) => hash,
                None => return None,
            };
        };

        if include_body {
            if let Some(block) = txn.get(&self.block_db, &block_hash) {
                chain_info.head = block;
            } else {
                warn!("Block body requested but not present");
            }
        }

        Some(chain_info)
    }

    pub fn get_block(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Block> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        if include_body {
            txn.get(&self.block_db, hash)
        } else {
            txn.get(&self.chain_db, hash)
                .map(|chain_info: ChainInfo| chain_info.head)
        }
    }

    pub fn get_block_at(
        &self,
        block_height: u32,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Block> {
        self.get_chain_info_at(block_height, include_body, txn_option)
            .map(|chain_info| chain_info.head)
    }

    pub fn get_blocks_at(
        &self,
        block_height: u32,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Vec<Block> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        // Seek to the first block at the given height.
        let mut blocks = Vec::new();
        let mut cursor = txn.cursor(&self.height_idx);
        let mut block_hash = match cursor.seek_key::<u32, Blake2bHash>(&block_height) {
            Some(hash) => hash,
            None => return blocks,
        };

        // Iterate until we find all blocks at the same height.
        while let Some(block) = self.get_block(&block_hash, include_body, Some(txn)) {
            blocks.push(block);

            // Get next block hash
            block_hash = match cursor.next_duplicate::<u32, Blake2bHash>() {
                Some((_, hash)) => hash,
                None => break,
            };
        }

        blocks
    }

    pub fn get_blocks_backward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Vec<Block> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        let mut blocks = Vec::new();
        let start_block = match self.get_block(start_block_hash, false, Some(txn)) {
            Some(block) => block,
            None => return blocks,
        };

        let mut hash = start_block.parent_hash().clone();
        while (blocks.len() as u32) < count {
            if let Some(block) = self.get_block(&hash, include_body, Some(txn)) {
                hash = block.parent_hash().clone();
                blocks.push(block);
            } else {
                break;
            }
        }

        blocks
    }

    pub fn get_blocks_forward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Vec<Block> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        let mut blocks = Vec::new();
        let mut chain_info = match self.get_chain_info(start_block_hash, false, Some(txn)) {
            Some(chain_info) => chain_info,
            None => return blocks,
        };

        while (blocks.len() as u32) < count {
            if let Some(ref successor) = chain_info.main_chain_successor {
                let chain_info_opt = self.get_chain_info(successor, include_body, Some(txn));
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

    pub fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        txn_option: Option<&Transaction>,
    ) -> Vec<Block> {
        match direction {
            Direction::Forward => {
                self.get_blocks_forward(start_block_hash, count, include_body, txn_option)
            }
            Direction::Backward => {
                self.get_blocks_backward(start_block_hash, count, include_body, txn_option)
            }
        }
    }

    /// Returns None if given start_block_hash is not a macro block.
    pub fn get_macro_blocks_backward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        election_blocks_only: bool,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<Block>> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        let mut blocks = Vec::new();
        let start_block = match self.get_block(start_block_hash, false, Some(txn)) {
            Some(Block::Macro(block)) => block,
            Some(_) => return None,
            None => return Some(blocks),
        };

        let mut hash = if election_blocks_only {
            start_block.header.parent_election_hash
        } else {
            start_block.header.parent_hash
        };
        while (blocks.len() as u32) < count {
            let block_opt = self.get_block(&hash, include_body, Some(txn));
            if let Some(Block::Macro(block)) = block_opt {
                hash = if election_blocks_only {
                    block.header.parent_election_hash.clone()
                } else {
                    block.header.parent_hash.clone()
                };
                blocks.push(Block::Macro(block));
            } else {
                break;
            }
        }

        Some(blocks)
    }

    /// Returns None if given start_block_hash is not a macro block.
    pub fn get_macro_blocks_forward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        election_blocks_only: bool,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<Block>> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        let mut blocks = Vec::new();
        let block = match self.get_block(start_block_hash, false, Some(txn)) {
            Some(Block::Macro(block)) => block,
            Some(_) => return None,
            None => return Some(blocks),
        };

        let mut next_macro_block = if election_blocks_only {
            policy::election_block_after(block.header.block_number)
        } else {
            policy::macro_block_after(block.header.block_number)
        };
        while (blocks.len() as u32) < count {
            let block_opt = self.get_block_at(next_macro_block, include_body, Some(txn));
            if let Some(Block::Macro(block)) = block_opt {
                next_macro_block = if election_blocks_only {
                    policy::election_block_after(block.header.block_number)
                } else {
                    policy::macro_block_after(block.header.block_number)
                };
                blocks.push(Block::Macro(block));
            } else {
                break;
            }
        }

        Some(blocks)
    }

    /// Returns None if given start_block_hash is not a macro block.
    pub fn get_macro_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        election_blocks_only: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<Block>> {
        match direction {
            Direction::Forward => self.get_macro_blocks_forward(
                start_block_hash,
                count,
                election_blocks_only,
                include_body,
                txn_option,
            ),
            Direction::Backward => self.get_macro_blocks_backward(
                start_block_hash,
                count,
                election_blocks_only,
                include_body,
                txn_option,
            ),
        }
    }

    pub fn put_receipts(&self, txn: &mut WriteTransaction, block_height: u32, receipts: &Receipts) {
        txn.put_reserve(&self.receipt_db, &block_height, receipts);
    }

    pub fn get_receipts(
        &self,
        block_height: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Receipts> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(&self.env);
                &read_txn
            }
        };

        txn.get(&self.receipt_db, &block_height)
    }

    pub fn clear_receipts(&self, txn: &mut WriteTransaction) {
        let mut cursor = txn.write_cursor(&self.receipt_db);
        let mut pos: Option<(u32, Receipts)> = cursor.first();

        while pos.is_some() {
            cursor.remove();
            pos = cursor.next();
        }
    }
}
