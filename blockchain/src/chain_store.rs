use crate::history::utils::IndexedValue;
use nimiq_account::RevertInfo;
use nimiq_block::Block;
use nimiq_blockchain_interface::{BlockchainError, ChainInfo, Direction};
use nimiq_database::{
    traits::{Database, ReadCursor, ReadTransaction, WriteCursor, WriteTransaction},
    DatabaseProxy, TableFlags, TableProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{policy::Policy, trie::trie_diff::TrieDiff};

type ChainInfoWrapper = IndexedValue<u32, ChainInfo>;

#[derive(Debug)]
pub struct ChainStore {
    /// Database handle.
    db: DatabaseProxy,
    /// A database of chain infos (it excludes the block body) indexed by their block hashes.
    chain_table: TableProxy,
    /// A database of block bodies indexed by their block hashes.
    block_table: TableProxy,
    /// A database of block hashes indexed by their block number.
    height_idx: TableProxy,
    /// A database of revert infos indexed by their corresponding block hashes.
    revert_table: TableProxy,
    /// A database of accounts trie diffs for a block.
    accounts_diff_table: TableProxy,
}

impl ChainStore {
    const CHAIN_DB_NAME: &'static str = "ChainData";
    const BLOCK_DB_NAME: &'static str = "Block";
    const HEIGHT_IDX_NAME: &'static str = "HeightIndex";
    const REVERT_DB_NAME: &'static str = "Receipts";
    const ACCOUNTS_DIFF_DB_NAME: &'static str = "AccountsDiff";

    const HEAD_KEY: &'static str = "head";

    pub fn new(db: DatabaseProxy) -> Self {
        let chain_table = db.open_table(Self::CHAIN_DB_NAME.to_string());
        let block_table =
            db.open_table_with_flags(Self::BLOCK_DB_NAME.to_string(), TableFlags::UINT_KEYS);
        let height_idx = db.open_table_with_flags(
            Self::HEIGHT_IDX_NAME.to_string(),
            TableFlags::DUPLICATE_KEYS | TableFlags::DUP_FIXED_SIZE_VALUES | TableFlags::UINT_KEYS,
        );
        let revert_table =
            db.open_table_with_flags(Self::REVERT_DB_NAME.to_string(), TableFlags::UINT_KEYS);
        let accounts_diff_table = db.open_table(Self::ACCOUNTS_DIFF_DB_NAME.to_string());
        ChainStore {
            db,
            chain_table,
            block_table,
            height_idx,
            revert_table,
            accounts_diff_table,
        }
    }

    pub fn clear(&self, txn: &mut WriteTransactionProxy) {
        txn.clear_database(&self.chain_table);
        txn.clear_database(&self.block_table);
        txn.clear_database(&self.height_idx);
        txn.clear_database(&self.revert_table);
        txn.clear_database(&self.accounts_diff_table);
    }

    pub fn get_head(&self, txn_option: Option<&TransactionProxy>) -> Option<Blake2bHash> {
        match txn_option {
            Some(txn) => txn.get(&self.chain_table, ChainStore::HEAD_KEY),
            None => self
                .db
                .read_transaction()
                .get(&self.chain_table, ChainStore::HEAD_KEY),
        }
    }

    pub fn set_head(&self, txn: &mut WriteTransactionProxy, hash: &Blake2bHash) {
        txn.put(&self.chain_table, ChainStore::HEAD_KEY, hash);
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
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        let wrapper: ChainInfoWrapper = match txn.get(&self.chain_table, hash) {
            Some(data) => data,
            None => return Err(BlockchainError::BlockNotFound),
        };

        let mut chain_info = wrapper.value;

        if include_body {
            if let Some(block) = txn.get(&self.block_table, &wrapper.index) {
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
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        // Seek to the first block at the given height.
        let cursor = txn.cursor(&self.height_idx);
        let block_hash_iter = cursor
            .into_iter_dup_of::<u32, Blake2bHash>(&block_height)
            .map(|(_height, hash)| hash);

        // Iterate until we find the main chain block.
        let mut wrapper = None;
        for tmp_block_hash in block_hash_iter {
            let tmp_wrapper: ChainInfoWrapper = txn
                .get(&self.chain_table, &tmp_block_hash)
                .expect("Corrupted store: ChainInfo referenced from index not found");

            let tmp_chain_info = &tmp_wrapper.value;

            // If it's on the main chain we can return from loop
            if tmp_chain_info.on_main_chain {
                wrapper = Some(tmp_wrapper);
                break;
            }
        }
        let wrapper = wrapper.ok_or(BlockchainError::BlockNotFound)?;
        let mut chain_info = wrapper.value;

        if include_body {
            if let Some(block) = txn.get(&self.block_table, &wrapper.index) {
                chain_info.head = block;
            } else {
                warn!(
                    index = wrapper.index,
                    "Block body requested but not present"
                );
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
        // Seek to the last leaf index of the block, if it exists.
        let mut cursor = WriteTransaction::cursor(txn, &self.block_table);
        let last_block_index = cursor.last::<u32, Block>().map(|(key, _)| key);

        let block_index = if let Some(index) = last_block_index {
            let wrapper: Option<ChainInfoWrapper> = txn.get(&self.chain_table, hash);
            if let Some(wrapper) = wrapper {
                wrapper.index
            } else {
                index + 1
            }
        } else {
            0
        };

        // Store chain data. Block body will not be persisted because the serialization of ChainInfo
        // ignores the block body.
        let wrapper = ChainInfoWrapper {
            value: chain_info.clone(),
            index: block_index,
        };
        txn.put(&self.chain_table, hash, &wrapper);

        // Store body if requested.
        if include_body {
            log::debug!(block_index, "Putting block");
            cursor.append(&block_index, &chain_info.head);
        }

        // Add to height index.
        let height = chain_info.head.block_number();
        txn.put(&self.height_idx, &height, hash);
    }

    /// Gets the set of macro block hashes that delimits the epoch chunks.
    /// For this it receives a block height and returns its corresponding hash
    /// plus all of the macro block hashes that were marked as non-prunable and
    /// that are part of the same epoch.
    pub fn get_epoch_chunks(
        &self,
        block_height: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<Vec<Blake2bHash>, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        let epoch_number = Policy::epoch_at(block_height);
        let mut blocks = vec![];
        let mut prev_macro_block_number = block_height;

        // Iterate until we find all non prunable macro blocks at the same epoch.
        while Policy::epoch_at(prev_macro_block_number) == epoch_number {
            match self.get_chain_info_at(prev_macro_block_number, false, Some(txn)) {
                Ok(chain_info) => {
                    // Only add non-prunable macro blocks and the block height passed as argument
                    if !chain_info.prunable || prev_macro_block_number == block_height {
                        blocks.push(chain_info.head.hash());
                    }
                }
                Err(_) => {
                    // Ignore error since the chain info for this block could have been pruned already and we need to
                    // find if there is a previous non-prunable macro block
                }
            }
            prev_macro_block_number = Policy::macro_block_before(prev_macro_block_number);
        }
        Ok(blocks.into_iter().rev().collect())
    }

    pub fn remove_chain_info(
        &self,
        txn: &mut WriteTransactionProxy,
        hash: &Blake2bHash,
        height: u32,
    ) {
        let wrapper: ChainInfoWrapper = txn
            .get(&self.chain_table, hash)
            .expect("Corrupted store: ChainInfo referenced from index not found");
        txn.remove(&self.chain_table, hash);
        txn.remove(&self.block_table, &wrapper.index);
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
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        let wrapper: ChainInfoWrapper = txn
            .get(&self.chain_table, hash)
            .ok_or(BlockchainError::BlockNotFound)?;

        if include_body {
            txn.get(&self.block_table, &wrapper.index)
                .ok_or(BlockchainError::BlockNotFound)
        } else {
            Ok(wrapper.value.head)
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
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        // Seek the hash of the first block at the given height and add it to our hashes vector
        let cursor = txn.cursor(&self.height_idx);

        cursor
            .into_iter_dup_of::<_, Blake2bHash>(&block_height)
            .map(|(_, hash)| hash)
            .collect()
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
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        // Iterate all blocks at the given height.
        let mut blocks = Vec::new();
        let cursor = txn.cursor(&self.height_idx);
        let iter = cursor.into_iter_dup_of(&block_height).map(|(_, hash)| hash);

        for block_hash in iter {
            blocks.push(
                self.get_block(&block_hash, include_body, Some(txn))
                    .unwrap_or_else(|_| {
                        panic!(
                            "Corrupted store: Block {} referenced from index not found",
                            block_hash
                        )
                    }),
            );
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
                read_txn = self.db.read_transaction();
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

    /// Does not include the block with `start_block_hash`.
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
                read_txn = self.db.read_transaction();
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
                read_txn = self.db.read_transaction();
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
                read_txn = self.db.read_transaction();
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
                let wrapper: ChainInfoWrapper = txn
                    .get(&self.chain_table, &hash)
                    .expect("Corrupted store: ChainInfo referenced from index not found");
                // If we detect a block whose prunable flag is set to false, we don't prune it
                // Then we need to keep the previous macro block
                if wrapper.value.prunable {
                    txn.remove(&self.chain_table, &hash);
                    txn.remove(&self.block_table, &wrapper.index);
                    txn.remove_item(&self.height_idx, &height, &hash);
                }
            }
        }
    }

    pub fn put_revert_info(
        &self,
        txn: &mut WriteTransactionProxy,
        block_height: u32,
        receipts: &RevertInfo,
    ) {
        txn.put_reserve(&self.revert_table, &block_height, receipts);
    }

    pub fn get_revert_info(
        &self,
        block_height: u32,
        txn_option: Option<&TransactionProxy>,
    ) -> Option<RevertInfo> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        txn.get(&self.revert_table, &block_height)
    }

    pub fn clear_revert_infos(&self, txn: &mut WriteTransactionProxy) {
        let mut cursor = WriteTransaction::cursor(txn, &self.revert_table);
        let mut pos: Option<(u32, RevertInfo)> = cursor.first();

        while pos.is_some() {
            cursor.remove();
            pos = cursor.next();
        }
    }

    pub fn put_accounts_diff(
        &self,
        txn: &mut WriteTransactionProxy,
        hash: &Blake2bHash,
        diff: &TrieDiff,
    ) {
        txn.put_reserve(&self.accounts_diff_table, hash, diff);
    }

    pub fn get_accounts_diff(
        &self,
        hash: &Blake2bHash,
        txn_option: Option<&TransactionProxy>,
    ) -> Result<TrieDiff, BlockchainError> {
        let read_txn: TransactionProxy;
        let txn = match txn_option {
            Some(txn) => txn,
            None => {
                read_txn = self.db.read_transaction();
                &read_txn
            }
        };

        match txn.get(&self.accounts_diff_table, hash) {
            Some(data) => Ok(data),
            None => {
                // Check if we know the block.
                let _ = self.get_block(hash, false, Some(txn))?;
                Err(BlockchainError::AccountsDiffNotFound)
            }
        }
    }
}
