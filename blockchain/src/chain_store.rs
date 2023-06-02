use nimiq_account::Receipts;
use nimiq_block::Block;
use nimiq_blockchain_interface::{BlockchainError, ChainInfo, Direction};
use nimiq_database::{
    traits::{Database, ReadCursor, ReadTransaction, WriteCursor, WriteTransaction},
    DatabaseProxy, TableFlags, TableProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy::Policy;

#[derive(Debug)]
pub struct ChainStore {
    env: DatabaseProxy,
    // A database of chain infos (it excludes the block body) indexed by their block hashes.
    chain_db: TableProxy,
    // A database of block bodies indexed by their block hashes.
    block_db: TableProxy,
    // A database of block hashes indexed by their block number.
    height_idx: TableProxy,
    // A database of the transaction receipts for a block, by their corresponding block hashes.
    receipt_db: TableProxy,
}

impl ChainStore {
    const CHAIN_DB_NAME: &'static str = "ChainData";
    const BLOCK_DB_NAME: &'static str = "Block";
    const HEIGHT_IDX_NAME: &'static str = "HeightIndex";
    const RECEIPT_DB_NAME: &'static str = "Receipts";

    const HEAD_KEY: &'static str = "head";

    pub fn new(env: DatabaseProxy) -> Self {
        let chain_db = env.open_table(Self::CHAIN_DB_NAME.to_string());
        let block_db = env.open_table(Self::BLOCK_DB_NAME.to_string());
        let height_idx = env.open_table_with_flags(
            Self::HEIGHT_IDX_NAME.to_string(),
            TableFlags::DUPLICATE_KEYS | TableFlags::DUP_FIXED_SIZE_VALUES | TableFlags::UINT_KEYS,
        );
        let receipt_db =
            env.open_table_with_flags(Self::RECEIPT_DB_NAME.to_string(), TableFlags::UINT_KEYS);
        ChainStore {
            env,
            chain_db,
            block_db,
            height_idx,
            receipt_db,
        }
    }

    pub fn clear(&self, txn: &mut WriteTransactionProxy) {
        txn.clear_database(&self.chain_db);
        txn.clear_database(&self.block_db);
        txn.clear_database(&self.height_idx);
        txn.clear_database(&self.receipt_db);
    }

    pub fn get_head(&self, txn_option: Option<&TransactionProxy>) -> Option<Blake2bHash> {
        match txn_option {
            Some(txn) => txn.get(&self.chain_db, ChainStore::HEAD_KEY),
            None => self
                .env
                .read_transaction()
                .get(&self.chain_db, ChainStore::HEAD_KEY),
        }
    }

    pub fn set_head(&self, txn: &mut WriteTransactionProxy, hash: &Blake2bHash) {
        txn.put(&self.chain_db, ChainStore::HEAD_KEY, hash);
    }

    pub fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<ChainInfo, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        let mut chain_info: ChainInfo = match txn.get(&self.chain_db, hash) {
            Some(data) => data,
            None => return Err(BlockchainError::BlockNotFound),
        };

        if include_body {
            if let Some(block) = txn.get(&self.block_db, hash) {
                chain_info.head = block;
            } else {
                warn!("Block body requested but not present");
            }
        }

        Ok(chain_info)
    }

    pub fn get_chain_info_at(
        &self,
        block_height: u32,
        include_body: bool,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<ChainInfo, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        // Seek to the first block at the given height.
        let mut cursor = txn.cursor(&self.height_idx);
        let mut block_hash = cursor
            .seek_key::<u32, Blake2bHash>(&block_height)
            .ok_or(BlockchainError::BlockNotFound)?;

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
                None => return Err(BlockchainError::BlockNotFound),
            };
        };

        if include_body {
            if let Some(block) = txn.get(&self.block_db, &block_hash) {
                chain_info.head = block;
            } else {
                warn!("Block body requested but not present");
            }
        }

        Ok(chain_info)
    }

    pub fn put_chain_info(
        &self,
        txn: &mut WriteTransactionProxy,
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

    pub fn get_epoch_chunks(
        &self,
        block_height: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Vec<Blake2bHash>, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        let block = self.get_block_at(block_height, false, Some(txn))?;
        let epoch_number = Policy::epoch_at(block_height);

        // Seek to the first block at the given height.
        let mut blocks = vec![block.hash()];
        let mut cursor = txn.cursor(&self.height_idx);
        // First block is the last one inserted
        let (first_block_number, _) = cursor
            .first::<u32, Blake2bHash>()
            .ok_or(BlockchainError::BlockNotFound)?;
        let mut last_block_number = block_height;

        // Iterate until we find all blocks at the same epoch.
        while Policy::epoch_at(last_block_number) == epoch_number
            && last_block_number != first_block_number
        {
            match cursor.prev_no_duplicate::<u32, Blake2bHash>() {
                Some((block_number, hash)) => {
                    if Policy::is_macro_block_at(block_number)
                        && Policy::epoch_at(block_number) == epoch_number
                    {
                        if let Ok(chain_info) = self.get_chain_info(&hash, false, Some(txn)) {
                            if !chain_info.prunable {
                                blocks.push(hash);
                            }
                        }
                    }
                    last_block_number = block_number;
                }
                None => break,
            }
        }
        Ok(blocks.into_iter().rev().collect())
    }

    pub fn remove_chain_info(
        &self,
        txn: &mut WriteTransactionProxy,
        hash: &Blake2bHash,
        height: u32,
    ) {
        txn.remove(&self.chain_db, hash);
        txn.remove(&self.block_db, hash);
        txn.remove_item(&self.height_idx, &height, hash);
    }

    pub fn get_block(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Block, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        if include_body {
            txn.get(&self.block_db, hash)
                .ok_or(BlockchainError::BlockNotFound)
        } else {
            txn.get(&self.chain_db, hash)
                .map(|chain_info: ChainInfo| chain_info.head)
                .ok_or(BlockchainError::BlockNotFound)
        }
    }

    pub fn get_block_at(
        &self,
        block_height: u32,
        include_body: bool,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Block, BlockchainError> {
        self.get_chain_info_at(block_height, include_body, txn_option)
            .map(|chain_info| chain_info.head)
    }

    pub fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Vec<Block>, BlockchainError> {
        match direction {
            Direction::Forward => {
                self.get_blocks_forward(start_block_hash, count, include_body, txn_option)
            }
            Direction::Backward => {
                self.get_blocks_backward(start_block_hash, count, include_body, txn_option)
            }
        }
    }

    pub fn get_block_hashes_at(
        &self,
        block_height: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Vec<Blake2bHash> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        let mut hashes: Vec<Blake2bHash> = Vec::new();
        // Seek the hash of the first block at the given height and add it to our hashes vector
        let mut cursor = txn.cursor(&self.height_idx);
        let mut hash = cursor
            .seek_key::<u32, Blake2bHash>(&block_height)
            .map(|hash| (0u32, hash));

        // Get any other block hash at the given height and add them to our hashes vector
        while let Some((_, h)) = hash {
            hashes.push(h);
            hash = cursor.next_duplicate::<u32, Blake2bHash>();
        }

        hashes
    }

    pub fn get_blocks_at(
        &self,
        block_height: u32,
        include_body: bool,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Vec<Block>, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        // Seek to the first block at the given height.
        let mut blocks = Vec::new();
        let mut cursor = txn.cursor(&self.height_idx);
        let mut block_hash = match cursor.seek_key::<u32, Blake2bHash>(&block_height) {
            Some(hash) => hash,
            None => return Err(BlockchainError::BlockNotFound),
        };

        // Iterate until we find all blocks at the same height.
        while let Ok(block) = self.get_block(&block_hash, include_body, Some(txn)) {
            blocks.push(block);

            // Get next block hash.
            block_hash = match cursor.next_duplicate::<u32, Blake2bHash>() {
                Some((_, hash)) => hash,
                None => break,
            };
        }

        Ok(blocks)
    }

    fn get_blocks_backward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Vec<Block>, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        let mut blocks = Vec::new();
        let start_block = self.get_block(start_block_hash, false, Some(txn))?;

        let mut hash = start_block.parent_hash().clone();
        while (blocks.len() as u32) < count {
            if let Ok(block) = self.get_block(&hash, include_body, Some(txn)) {
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
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Vec<Block>, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        let mut blocks = Vec::new();
        let mut chain_info = self.get_chain_info(start_block_hash, false, Some(txn))?;

        while (blocks.len() as u32) < count {
            if let Some(ref successor) = chain_info.main_chain_successor {
                let chain_info_opt = self.get_chain_info(successor, include_body, Some(txn));
                if chain_info_opt.is_err() {
                    break;
                }

                chain_info = chain_info_opt.unwrap();
                blocks.push(chain_info.head);
            } else {
                break;
            }
        }

        Ok(blocks)
    }

    /// Returns None if given start_block_hash is not a macro block.
    pub fn get_macro_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        election_blocks_only: bool,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Vec<Block>, BlockchainError> {
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

    fn get_macro_blocks_backward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        election_blocks_only: bool,
        include_body: bool,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Vec<Block>, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        let mut blocks = Vec::new();
        let start_block = match self.get_block(start_block_hash, false, Some(txn)) {
            Ok(Block::Macro(block)) => block,
            Ok(_) => return Err(BlockchainError::BlockIsNotMacro),
            Err(e) => return Err(e),
        };

        let mut hash = if election_blocks_only {
            start_block.header.parent_election_hash
        } else {
            start_block.header.parent_hash
        };
        while (blocks.len() as u32) < count {
            let block_result = self.get_block(&hash, include_body, Some(txn));
            if let Ok(Block::Macro(block)) = block_result {
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

        Ok(blocks)
    }

    /// Returns None if given start_block_hash is not a macro block.
    fn get_macro_blocks_forward(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        election_blocks_only: bool,
        include_body: bool,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Vec<Block>, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        let mut blocks = Vec::new();
        let block = match self.get_block(start_block_hash, false, Some(txn)) {
            Ok(Block::Macro(block)) => block,
            Ok(_) => return Err(BlockchainError::BlockIsNotMacro),
            Err(e) => return Err(e),
        };

        let mut next_macro_block = if election_blocks_only {
            Policy::election_block_after(block.header.block_number)
        } else {
            Policy::macro_block_after(block.header.block_number)
        };
        while (blocks.len() as u32) < count {
            let block_result = self.get_block_at(next_macro_block, include_body, Some(txn));
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
                Err(BlockchainError::BlockNotFound) => break,
                Err(e) => return Err(e),
            }
        }

        Ok(blocks)
    }

    pub fn prune_epoch(&self, epoch_number: u32, txn: &mut WriteTransactionProxy) {
        // The zero-th epoch is already pruned.
        if epoch_number == 0 {
            return;
        }

        for height in Policy::first_block_of(epoch_number)
            .expect("The supplied epoch_number is out of bounds")
            ..Policy::election_block_of(epoch_number)
                .expect("The supplied epoch_number is out of bounds")
        {
            let hashes = self.get_block_hashes_at(height, Some(txn));
            for hash in hashes {
                let chain_info: ChainInfo = txn
                    .get(&self.chain_db, &hash)
                    .expect("Corrupted store: ChainInfo referenced from index not found");
                // If we detect a block whose prunable flag is set to false, we don't prune it
                // Then we need to keep the previous macro block
                if chain_info.prunable {
                    txn.remove(&self.chain_db, &hash);
                    txn.remove(&self.block_db, &hash);
                    txn.remove_item(&self.height_idx, &height, &hash);
                }
            }
        }
    }

    pub fn put_receipts(
        &self,
        txn: &mut WriteTransactionProxy,
        block_height: u32,
        receipts: &Receipts,
    ) {
        txn.put_reserve(&self.receipt_db, &block_height, receipts);
    }

    pub fn get_receipts(
        &self,
        block_height: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<Receipts> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.env.read_transaction();
                &read_txn
            }
        };

        txn.get(&self.receipt_db, &block_height)
    }

    pub fn clear_receipts(&self, txn: &mut WriteTransactionProxy) {
        let mut cursor = WriteTransaction::cursor(txn, &self.receipt_db);
        let mut pos: Option<(u32, Receipts)> = cursor.first();

        while pos.is_some() {
            cursor.remove();
            pos = cursor.next();
        }
    }
}
