use consensus::base::block::Block;
use consensus::base::blockchain::ChainData;
use consensus::base::primitive::hash::Blake2bHash;
use utils::db::{Environment, Database, DatabaseFlags, Transaction, ReadTransaction, WriteTransaction};

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
        return ChainStore { env, chain_db, block_db, height_idx };
    }

    pub fn get_head(&self, txn_option: Option<&Transaction>) -> Option<Blake2bHash> {
        return match txn_option {
            Some(txn) => txn.get(&self.chain_db, ChainStore::HEAD_KEY),
            None => ReadTransaction::new(self.env).get(&self.chain_db, ChainStore::HEAD_KEY)
        };
    }

    pub fn set_head(&self, txn: &mut WriteTransaction, hash: &Blake2bHash) {
        txn.put(&self.chain_db, ChainStore::HEAD_KEY, hash);
    }

    pub fn get_chain_data(&self, hash: &Blake2bHash, include_body: bool, txn_option: Option<&Transaction>) -> Option<ChainData> {
        let read_txn: ReadTransaction;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = ReadTransaction::new(self.env);
                &read_txn
            }
        };

        let mut chain_data: ChainData = match txn.get(&self.chain_db, hash) {
            Some(data) => data,
            None => return None
        };

        if include_body {
            if let Some(block) = txn.get(&self.block_db, hash) {
                chain_data.head = block;
            } else {
                warn!("Block body requested but not present");
            }
        }

        return Some(chain_data);
    }

    pub fn put_chain_data(&self, txn: &mut WriteTransaction, hash: &Blake2bHash, chain_data: &ChainData, include_body: bool) {
        // Store chain data. Block body will not be persisted.
        txn.put_reserve(&self.chain_db, hash, chain_data);

        // Store body if requested.
        if include_body && chain_data.head.body.is_some() {
            txn.put_reserve(&self.block_db, hash, &chain_data.head);
        }

        // Add to height index.
        let height = chain_data.head.header.height;
        txn.put(&self.height_idx, &height, hash);
    }

    pub fn get_chain_data_at(&self, block_height: u32, include_body: bool, txn_option: Option<&Transaction>) -> Option<ChainData> {
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
        let mut chain_data: ChainData;
        while {
            // Loop condition
            chain_data = txn
                .get(&self.chain_db, &block_hash)
                .expect("Corrupted store: ChainData referenced from index not found");
            !chain_data.on_main_chain
        } {
            // Loop Body
            block_hash = match cursor.next_duplicate::<u32, Blake2bHash>() {
                Some((_, hash)) => hash,
                None => return None
            };
        }

        if include_body {
            if let Some(block) = txn.get(&self.block_db, &block_hash) {
                chain_data.head = block;
            } else {
                warn!("Block body requested but not present");
            }
        }

        return Some(chain_data);
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

        return if include_body {
            txn.get(&self.block_db, hash)
        } else {
            txn.get(&self.chain_db, hash).map(|chain_data: ChainData| chain_data.head)
        };
    }

    pub fn get_block_at(&self, block_height: u32) -> Option<Block> {
        unimplemented!();
    }

    pub fn get_blocks_backward(&self, start_block_hash: &Blake2bHash, count: u32, include_body: bool) -> Vec<Block> {
        let txn = ReadTransaction::new(self.env);

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

        return blocks;
    }
}
