use parking_lot::{MappedRwLockReadGuard, MutexGuard, RwLockReadGuard};

use account::{Account, StakingContract};
use block::{Block, BlockType, MacroBlock};
use database::{Transaction, WriteTransaction};
use genesis::NetworkInfo;
use hash::Blake2bHash;
use keys::Address;
use primitives::policy;
use primitives::slot::ValidatorSlots;
use transaction::{Transaction as BlockchainTransaction, TransactionReceipt};
use utils::observer::{Listener, ListenerHandle};

use crate::blockchain_state::BlockchainState;
#[cfg(feature = "metrics")]
use crate::chain_metrics::BlockchainMetrics;
use crate::history_store::{ExtTxData, HistoryTreeChunk};
use crate::{Blockchain, BlockchainEvent, Direction};

/// Implements several wrapper functions.
impl Blockchain {
    /// Returns the head of the main chain.
    pub fn head(&self) -> MappedRwLockReadGuard<Block> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| &s.main_chain.head)
    }

    /// Returns the last macro block.
    pub fn macro_head(&self) -> MappedRwLockReadGuard<MacroBlock> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| s.macro_info.head.unwrap_macro_ref())
    }

    /// Returns the last election macro block.
    pub fn election_head(&self) -> MappedRwLockReadGuard<MacroBlock> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| &s.election_head)
    }

    /// Returns the hash of the head of the main chain.
    pub fn head_hash(&self) -> Blake2bHash {
        self.state.read().head_hash.clone()
    }

    /// Returns the hash of the last macro block.
    pub fn macro_head_hash(&self) -> Blake2bHash {
        self.state.read().macro_head_hash.clone()
    }

    /// Returns the hash of the last election macro block.
    pub fn election_head_hash(&self) -> Blake2bHash {
        self.state.read().election_head_hash.clone()
    }

    /// Returns the block number at the head of the main chain.
    pub fn block_number(&self) -> u32 {
        self.state.read_recursive().main_chain.head.block_number()
    }

    /// Returns the timestamp at the head of the main chain.
    pub fn timestamp(&self) -> u64 {
        self.state.read_recursive().main_chain.head.timestamp()
    }

    /// Returns the view number at the head of the main chain.
    pub fn view_number(&self) -> u32 {
        self.state.read_recursive().main_chain.head.view_number()
    }

    /// Returns the next view number at the head of the main chain.
    pub fn next_view_number(&self) -> u32 {
        self.state
            .read_recursive()
            .main_chain
            .head
            .next_view_number()
    }

    /// Returns the current set of validators.
    pub fn current_validators(&self) -> MappedRwLockReadGuard<ValidatorSlots> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| s.current_validators().unwrap())
    }

    /// Returns the set of validators of the previous epoch.
    pub fn last_validators(&self) -> MappedRwLockReadGuard<ValidatorSlots> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| s.last_validators().unwrap())
    }

    /// Returns the current state (with a read transaction).
    pub fn state(&self) -> RwLockReadGuard<BlockchainState> {
        self.state.read()
    }

    /// Checks if the blockchain contains a specific block, by its hash.
    pub fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool {
        match self.chain_store.get_chain_info(hash, false, None) {
            Some(chain_info) => include_forks || chain_info.on_main_chain,
            None => false,
        }
    }

    /// Returns the block type of the next block.
    pub fn get_next_block_type(&self, last_number: Option<u32>) -> BlockType {
        let last_block_number = match last_number {
            None => self.head().block_number(),
            Some(n) => n,
        };

        if policy::is_macro_block_at(last_block_number + 1) {
            BlockType::Macro
        } else {
            BlockType::Micro
        }
    }

    /// Fetches a given block, by its block number.
    pub fn get_block_at(&self, height: u32, include_body: bool) -> Option<Block> {
        self.chain_store.get_block_at(height, include_body, None)
    }

    /// Fetches a given block, by its hash.
    pub fn get_block(&self, hash: &Blake2bHash, include_body: bool) -> Option<Block> {
        self.chain_store.get_block(hash, include_body, None)
    }

    /// Fetches a given number of blocks, starting at a specific block (by its hash).
    pub fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
    ) -> Vec<Block> {
        self.chain_store
            .get_blocks(start_block_hash, count, include_body, direction, None)
    }

    /// Fetches a given number of macro blocks, starting at a specific block (by its hash).
    /// It can fetch only election macro blocks.
    /// Returns None if given start_block_hash is not a macro block.
    pub fn get_macro_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        election_blocks_only: bool,
    ) -> Option<Vec<Block>> {
        self.chain_store.get_macro_blocks(
            start_block_hash,
            count,
            include_body,
            direction,
            election_blocks_only,
            None,
        )
    }

    /// Returns a list of all transactions for either an epoch or a batch.
    fn get_transactions(
        &self,
        batch_or_epoch_index: u32,
        for_batch: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<BlockchainTransaction>> {
        if !for_batch {
            // It might be that we synced this epoch via macro block sync and don't actually have
            // the micro blocks.
            // Therefore, we check this first.
            let ext_txs = self
                .history_store
                .get_epoch_transactions(batch_or_epoch_index, txn_option)?;

            let mut txs = vec![];

            for ext_tx in ext_txs {
                if let ExtTxData::Basic(tx) = ext_tx.data {
                    txs.push(tx);
                }
            }

            return Some(txs);
        }

        // Else retrieve transactions normally from micro blocks.
        // first_block_of(_batch) is guaranteed to return a micro block!!!
        let first_block = if for_batch {
            policy::first_block_of_batch(batch_or_epoch_index)
        } else {
            policy::first_block_of(batch_or_epoch_index)
        };
        let first_block = self
            .chain_store
            .get_block_at(first_block, true, txn_option)
            .or_else(|| {
                debug!(
                    "get_block_at didn't return first block of {}: block_height={}",
                    if for_batch { "batch" } else { "epoch" },
                    first_block,
                );
                None
            })?;

        let first_hash = first_block.hash();
        let mut txs = first_block.unwrap_micro().body.unwrap().transactions;

        // Excludes current block and macro block.
        let blocks = self.chain_store.get_blocks(
            &first_hash,
            if for_batch {
                policy::BATCH_LENGTH
            } else {
                policy::EPOCH_LENGTH
            } - 2,
            true,
            Direction::Forward,
            txn_option,
        );

        txs.extend(
            blocks
                .into_iter()
                // blocks need to be filtered as Block::unwrap_transactions makes use of
                // Block::unwrap_micro, which panics for non micro blocks.
                .filter(Block::is_micro as fn(&_) -> _)
                .map(Block::unwrap_transactions as fn(_) -> _)
                .flatten(),
        );

        Some(txs)
    }

    /// Returns a list of all transactions for a given batch.
    pub fn get_batch_transactions(
        &self,
        batch: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<BlockchainTransaction>> {
        self.get_transactions(batch, true, txn_option)
    }

    /// Returns a list of all transactions for a given epoch.
    pub fn get_epoch_transactions(
        &self,
        epoch: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<BlockchainTransaction>> {
        self.get_transactions(epoch, false, txn_option)
    }

    /// Returns the history root for a given epoch.
    pub fn get_history_root(
        &self,
        epoch: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Blake2bHash> {
        self.history_store.get_history_tree_root(epoch, txn_option)
    }

    /// Returns the number of extended transactions for a given epoch.
    pub fn get_num_extended_transactions(
        &self,
        epoch_number: u32,
        txn_option: Option<&Transaction>,
    ) -> usize {
        self.history_store
            .get_num_extended_transactions(epoch_number, txn_option)
    }

    /// Returns the `chunk_index`th chunk of size `chunk_size` for a given epoch.
    /// The return value consists of a vector of all the extended transactions in that chunk
    /// and a proof for these in the MMR.
    pub fn get_chunk(
        &self,
        epoch_number: u32,
        chunk_size: usize,
        chunk_index: usize,
        txn_option: Option<&Transaction>,
    ) -> Option<HistoryTreeChunk> {
        self.history_store
            .get_chunk(epoch_number, chunk_size, chunk_index, txn_option)
    }

    /// Returns the current staking contract.
    pub fn get_staking_contract(&self) -> StakingContract {
        let validator_registry = NetworkInfo::from_network_id(self.network_id)
            .validator_registry_address()
            .expect("No ValidatorRegistry");

        let account = self.state.read().accounts().get(validator_registry, None);

        if let Account::Staking(x) = account {
            x
        } else {
            unreachable!("Account type must be Staking.")
        }
    }

    pub fn write_transaction(&self) -> WriteTransaction {
        WriteTransaction::new(&self.env)
    }

    pub fn register_listener<T: Listener<BlockchainEvent> + 'static>(
        &self,
        listener: T,
    ) -> ListenerHandle {
        self.notifier.write().register(listener)
    }

    pub fn get_account(&self, address: &Address) -> Account {
        self.state.read().accounts.get(address, None)
    }

    pub fn contains_tx_in_validity_window(&self, tx_hash: &Blake2bHash) -> bool {
        self.state.read().transaction_cache.contains(tx_hash)
    }

    pub fn validator_registry_address(&self) -> Option<&Address> {
        NetworkInfo::from_network_id(self.network_id).validator_registry_address()
    }

    pub fn lock(&self) -> MutexGuard<()> {
        self.push_lock.lock()
    }

    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> &BlockchainMetrics {
        &self.metrics
    }

    // TODO: Implement this method. It is used in the rpc-server.
    #[allow(unused_variables)]
    pub fn get_transaction_receipts_by_address(
        &self,
        address: &Address,
        sender_limit: usize,
        recipient_limit: usize,
    ) -> Vec<TransactionReceipt> {
        unimplemented!()
    }
}
