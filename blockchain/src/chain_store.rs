use std::sync::Arc;

use nimiq_account::RevertInfo;
use nimiq_block::{Block, BlockBody, BlockJustification, MacroBlock, MacroBody, MicroBlock};
use nimiq_blockchain_interface::{BlockchainError, ChainInfo, Direction};
use nimiq_database::{
    declare_table,
    mdbx::{MdbxDatabase, MdbxReadTransaction, MdbxWriteTransaction, OptionalTransaction},
    traits::{Database, DupReadCursor, ReadCursor, ReadTransaction, WriteCursor, WriteTransaction},
};
use nimiq_database_value_derive::DbSerializable;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{policy::Policy, trie::trie_diff::TrieDiff};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_transaction::{historic_transaction::HistoricTransactionData, reward::RewardTransaction};

use crate::history::interface::HistoryInterface;
use crate::history_store_proxy::HistoryStoreProxy;

declare_table!(HeadTable, "Head", () => Blake2bHash);
declare_table!(ChainTable, "ChainData", Blake2bHash => ChainInfo);
declare_table!(PushedBlockTable, "PushedBlockTable", Blake2bHash => PushedBlock);
declare_table!(StoredBlockTable, "StoredBlockTable", Blake2bHash => Block);
declare_table!(HeightIndex, "HeightIndex", u32 => dup(Blake2bHash));
declare_table!(RevertTable, "Receipts", u32 => RevertInfo);
declare_table!(AccountsDiffTable, "AccountsDiff", Blake2bHash => TrieDiff);

/// The non-header content of a block except that transactions are not stored to
/// optimize blocks storage. This assumes that a block has been pushed and that there
/// is history associated with this block.
#[derive(Debug, Serialize, Deserialize, DbSerializable)]
pub struct PushedBlock {
    justification: Option<BlockJustification>,
    #[serde(serialize_with = "PushedBlock::serialize_body")]
    body: Option<BlockBody>,
}

impl From<Block> for PushedBlock {
    fn from(block: Block) -> Self {
        PushedBlock {
            justification: block.justification(),
            body: block.body(),
        }
    }
}

impl PushedBlock {
    fn block_from_header(
        &self,
        block: &Block,
        history_store: &Arc<HistoryStoreProxy>,
        txn: &MdbxReadTransaction,
    ) -> Block {
        if let Some(justification) = &self.justification {
            assert!(block.ty() == justification.ty());
        }
        if let Some(body) = &self.body {
            assert!(block.ty() == body.ty());
        }
        let block_to_return = match block {
            Block::Macro(header_only_block) => Block::Macro(MacroBlock {
                header: header_only_block.header.clone(),
                justification: self
                    .justification
                    .as_ref()
                    .map(|justification| justification.clone().unwrap_macro()),
                body: self.body.as_ref().map(|_| {
                    let mut full_macro_body = MacroBody::default();
                    // Recover transactions from the history store
                    full_macro_body.transactions = history_store
                        .get_block_transactions(header_only_block.block_number(), Some(txn))
                        .iter()
                        .filter_map(|hist_txn| {
                            if matches!(hist_txn.data, HistoricTransactionData::Reward(_)) {
                                let reward_event = hist_txn.unwrap_reward();
                                Some(RewardTransaction {
                                    recipient: reward_event.reward_address.clone(),
                                    validator_address: reward_event.validator_address.clone(),
                                    value: reward_event.value,
                                })
                            } else {
                                None
                            }
                        })
                        .collect();
                    full_macro_body
                }),
            }),
            Block::Micro(header_only_block) => Block::Micro(MicroBlock {
                header: header_only_block.header.clone(),
                justification: self
                    .justification
                    .as_ref()
                    .map(|justification| justification.clone().unwrap_micro()),
                body: self.body.as_ref().map(|body| {
                    let mut full_micro_body = body.clone().unwrap_micro();
                    full_micro_body.transactions = history_store
                        .get_block_transactions(header_only_block.block_number(), Some(txn))
                        .iter()
                        .filter_map(|hist_txn| {
                            if !hist_txn.is_not_basic() {
                                Some(hist_txn.unwrap_basic().to_owned())
                            } else {
                                None
                            }
                        })
                        .collect();
                    full_micro_body
                }),
            }),
        };
        assert!(block_to_return.verify(block.network()).is_ok());
        block_to_return
    }

    fn serialize_body<S>(body: &Option<BlockBody>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Empty out body transactions if there is any: these are already stored in the History Store.
        if let Some(body) = body {
            let mut body_to_ser = body.clone();
            match body_to_ser {
                BlockBody::Micro(ref mut body) => body.transactions = vec![],
                BlockBody::Macro(ref mut body) => body.transactions = vec![],
            }
            serde::Serialize::serialize(&Some(body_to_ser), serializer)
        } else {
            serde::Serialize::serialize(&body, serializer)
        }
    }
}

#[derive(Debug)]
pub struct ChainStore {
    /// Database handle.
    db: MdbxDatabase,
    /// A database of the head block hash.
    head_table: HeadTable,
    /// A database of chain infos (it excludes the block body) indexed by their block hashes.
    chain_table: ChainTable,
    /// A database of block justification and bodies indexed by their block hashes.
    pushed_block_table: PushedBlockTable,
    /// A database of blocks indexed by their block hashes.
    stored_block_table: StoredBlockTable,
    /// A database of block hashes indexed by their block number.
    height_idx: HeightIndex,
    /// A database of revert infos indexed by their corresponding block hashes.
    revert_table: RevertTable,
    /// A database of accounts trie diffs for a block.
    accounts_diff_table: AccountsDiffTable,
    /// A reference to the history store
    history_store: Arc<HistoryStoreProxy>,
}

impl ChainStore {
    pub fn new(db: MdbxDatabase, history_store: Arc<HistoryStoreProxy>) -> Self {
        let chain_store = ChainStore {
            db,
            head_table: HeadTable,
            chain_table: ChainTable,
            pushed_block_table: PushedBlockTable,
            stored_block_table: StoredBlockTable,
            height_idx: HeightIndex,
            revert_table: RevertTable,
            accounts_diff_table: AccountsDiffTable,
            history_store,
        };

        chain_store.db.create_regular_table(&chain_store.head_table);
        chain_store
            .db
            .create_regular_table(&chain_store.chain_table);
        chain_store
            .db
            .create_regular_table(&chain_store.pushed_block_table);
        chain_store
            .db
            .create_regular_table(&chain_store.stored_block_table);
        chain_store.db.create_dup_table(&chain_store.height_idx);
        chain_store
            .db
            .create_regular_table(&chain_store.revert_table);
        chain_store
            .db
            .create_regular_table(&chain_store.accounts_diff_table);

        chain_store
    }

    pub fn clear(&self, txn: &mut MdbxWriteTransaction) {
        txn.clear_table(&self.chain_table);
        txn.clear_table(&self.pushed_block_table);
        txn.clear_table(&self.stored_block_table);
        txn.clear_table(&self.height_idx);
        txn.clear_table(&self.revert_table);
        txn.clear_table(&self.accounts_diff_table);
    }

    pub fn get_head(&self, txn_option: Option<&MdbxReadTransaction>) -> Option<Blake2bHash> {
        let txn = txn_option.or_new(&self.db);
        txn.get(&self.head_table, &())
    }

    pub fn set_head(&self, txn: &mut MdbxWriteTransaction, hash: &Blake2bHash) {
        txn.put(&self.head_table, &(), hash);
    }

    pub fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<ChainInfo, BlockchainError> {
        let txn = txn_option.or_new(&self.db);

        let mut chain_info: ChainInfo = match txn.get(&self.chain_table, hash) {
            Some(data) => data,
            None => return Err(BlockchainError::BlockNotFound),
        };

        if include_body {
            // Check the stored block table, then the pushed block table
            if let Some(full_block) = txn.get(&self.stored_block_table, hash) {
                chain_info.head = full_block
            } else if let Some(block) = txn.get(&self.pushed_block_table, hash) {
                chain_info.head =
                    block.block_from_header(&chain_info.head, &self.history_store, &txn);
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
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<ChainInfo, BlockchainError> {
        let txn = txn_option.or_new(&self.db);

        // Seek to the first block at the given height.
        let cursor = txn.dup_cursor(&self.height_idx);
        let block_hash_iter = cursor
            .into_iter_dup_of(&block_height)
            .map(|(_height, hash)| hash);

        // Iterate until we find the main chain block.
        let mut chain_info = None;
        let mut block_hash = None;
        for tmp_block_hash in block_hash_iter {
            let tmp_chain_info: ChainInfo = txn
                .get(&self.chain_table, &tmp_block_hash)
                .expect("Corrupted store: ChainInfo referenced from index not found");

            // If it's on the main chain we can return from loop
            if tmp_chain_info.on_main_chain {
                chain_info = Some(tmp_chain_info);
                block_hash = Some(tmp_block_hash);
                break;
            }
        }
        let mut chain_info = chain_info.ok_or(BlockchainError::BlockNotFound)?;
        let block_hash = block_hash.unwrap();

        if include_body {
            // Check the stored block table, then the pushed block table
            if let Some(full_block) = txn.get(&self.stored_block_table, &block_hash) {
                chain_info.head = full_block
            } else if let Some(block) = txn.get(&self.pushed_block_table, &block_hash) {
                chain_info.head =
                    block.block_from_header(&chain_info.head, &self.history_store, &txn);
            } else {
                warn!("Block body requested but not present");
            }
        }

        Ok(chain_info)
    }

    pub fn put_chain_info(
        &self,
        txn: &mut MdbxWriteTransaction,
        hash: &Blake2bHash,
        chain_info: &ChainInfo,
        include_body: bool,
        is_block_pushed: bool,
    ) {
        // Store chain data. Block body will not be persisted because the serialization of ChainInfo
        // ignores the block body.
        txn.put_reserve(&self.chain_table, hash, chain_info);

        // Store body if requested.
        if include_body {
            if is_block_pushed {
                txn.put_reserve(
                    &self.pushed_block_table,
                    hash,
                    &PushedBlock::from(chain_info.head.clone()),
                );
            } else {
                txn.put_reserve(&self.stored_block_table, hash, &chain_info.head);
            }
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
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<Vec<Blake2bHash>, BlockchainError> {
        let txn = txn_option.or_new(&self.db);

        let epoch_number = Policy::epoch_at(block_height);
        let mut blocks = vec![];
        let mut prev_macro_block_number = block_height;

        // Iterate until we find all non prunable macro blocks at the same epoch.
        while Policy::epoch_at(prev_macro_block_number) == epoch_number {
            match self.get_chain_info_at(prev_macro_block_number, false, Some(&txn)) {
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
        txn: &mut MdbxWriteTransaction,
        hash: &Blake2bHash,
        height: u32,
    ) {
        txn.remove(&self.chain_table, hash);
        txn.remove(&self.pushed_block_table, hash);
        txn.remove(&self.stored_block_table, hash);
        txn.remove_item(&self.height_idx, &height, hash);
    }

    pub fn get_block(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<Block, BlockchainError> {
        let txn = txn_option.or_new(&self.db);

        let chain_info = txn
            .get(&self.chain_table, hash)
            .ok_or(BlockchainError::BlockNotFound)?;

        if include_body {
            // Check first the store blocks table since it is smaller then the pushed blocks table
            if let Some(full_block) = txn.get(&self.stored_block_table, hash) {
                Ok(full_block)
            } else {
                let headerless_block = txn
                    .get(&self.pushed_block_table, hash)
                    .ok_or(BlockchainError::BlockNotFound)?;
                Ok(headerless_block.block_from_header(&chain_info.head, &self.history_store, &txn))
            }
        } else {
            Ok(chain_info.head)
        }
    }

    pub fn get_block_at(
        &self,
        block_height: u32,
        include_body: bool,
        txn_option: Option<&MdbxReadTransaction>,
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
        txn_option: Option<&MdbxReadTransaction>,
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
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Vec<Blake2bHash> {
        let txn = txn_option.or_new(&self.db);

        // Seek the hash of the first block at the given height and add it to our hashes vector
        let cursor = txn.dup_cursor(&self.height_idx);

        cursor
            .into_iter_dup_of(&block_height)
            .map(|(_, hash)| hash)
            .collect()
    }

    pub fn get_blocks_at(
        &self,
        block_height: u32,
        include_body: bool,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<Vec<Block>, BlockchainError> {
        let txn = txn_option.or_new(&self.db);

        // Iterate all blocks at the given height.
        let mut blocks = Vec::new();
        let cursor = txn.dup_cursor(&self.height_idx);
        let iter = cursor.into_iter_dup_of(&block_height).map(|(_, hash)| hash);

        for block_hash in iter {
            blocks.push(
                self.get_block(&block_hash, include_body, Some(&txn))
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
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<Vec<Block>, BlockchainError> {
        let txn = txn_option.or_new(&self.db);

        let mut blocks = Vec::new();
        let start_block = self.get_block(start_block_hash, false, Some(&txn))?;

        let mut hash = start_block.parent_hash().clone();
        while (blocks.len() as u32) < count {
            if let Ok(block) = self.get_block(&hash, include_body, Some(&txn)) {
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
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<Vec<Block>, BlockchainError> {
        let txn = txn_option.or_new(&self.db);

        let mut blocks = Vec::new();
        let mut chain_info = self.get_chain_info(start_block_hash, false, Some(&txn))?;

        while (blocks.len() as u32) < count {
            if let Some(ref successor) = chain_info.main_chain_successor {
                let chain_info_opt = self.get_chain_info(successor, include_body, Some(&txn));
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
        txn_option: Option<&MdbxReadTransaction>,
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
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<Vec<Block>, BlockchainError> {
        let txn = txn_option.or_new(&self.db);

        let mut blocks = Vec::new();
        let start_block = match self.get_block(start_block_hash, false, Some(&txn)) {
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
            let block_result = self.get_block(&hash, include_body, Some(&txn));
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
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<Vec<Block>, BlockchainError> {
        let txn = txn_option.or_new(&self.db);

        let mut blocks = Vec::new();
        let block = match self.get_block(start_block_hash, false, Some(&txn)) {
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
            let block_result = self.get_block_at(next_macro_block, include_body, Some(&txn));
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

    pub fn prune_epoch(&self, epoch_number: u32, txn: &mut MdbxWriteTransaction) {
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
                    .get(&self.chain_table, &hash)
                    .expect("Corrupted store: ChainInfo referenced from index not found");
                // If we detect a block whose prunable flag is set to false, we don't prune it
                // Then we need to keep the previous macro block
                if chain_info.prunable {
                    txn.remove(&self.chain_table, &hash);
                    txn.remove(&self.pushed_block_table, &hash);
                    txn.remove_item(&self.height_idx, &height, &hash);
                }
            }
        }

        // Clear the stored block table since epoch is finalized
        txn.clear_table(&self.stored_block_table);
    }

    /// Finalizes a batch by removing blocks that were only stored
    pub fn finalize_batch(&self, txn: &mut MdbxWriteTransaction) {
        let mut cursor = WriteTransaction::cursor(txn, &self.stored_block_table);
        let mut pos: Option<(Blake2bHash, Block)> = cursor.first();

        // Remove the item first from the height index table
        while let Some((hash, block)) = pos {
            txn.remove_item(&self.height_idx, &block.block_number(), &hash);
            pos = cursor.next();
        }
        // Then clear the stored block table
        txn.clear_table(&self.stored_block_table);
    }

    /// Puts a revert info for a block height
    pub fn put_revert_info(
        &self,
        txn: &mut MdbxWriteTransaction,
        block_height: u32,
        receipts: &RevertInfo,
    ) {
        txn.put_reserve(&self.revert_table, &block_height, receipts);
    }

    /// Gets the revert info for a particular block height
    pub fn get_revert_info(
        &self,
        block_height: u32,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Option<RevertInfo> {
        let txn = txn_option.or_new(&self.db);

        txn.get(&self.revert_table, &block_height)
    }

    pub fn clear_revert_infos(&self, txn: &mut MdbxWriteTransaction) {
        let mut cursor = WriteTransaction::cursor(txn, &self.revert_table);
        let mut pos: Option<(u32, RevertInfo)> = cursor.first();

        while pos.is_some() {
            cursor.remove();
            pos = cursor.next();
        }
    }

    pub fn put_accounts_diff(
        &self,
        txn: &mut MdbxWriteTransaction,
        hash: &Blake2bHash,
        diff: &TrieDiff,
    ) {
        txn.put_reserve(&self.accounts_diff_table, hash, diff);
    }

    pub fn get_accounts_diff(
        &self,
        hash: &Blake2bHash,
        txn_option: Option<&MdbxReadTransaction>,
    ) -> Result<TrieDiff, BlockchainError> {
        let txn = txn_option.or_new(&self.db);

        match txn.get(&self.accounts_diff_table, hash) {
            Some(data) => Ok(data),
            None => {
                // Check if we know the block.
                let _ = self.get_block(hash, false, Some(&txn))?;
                Err(BlockchainError::AccountsDiffNotFound)
            }
        }
    }
}
