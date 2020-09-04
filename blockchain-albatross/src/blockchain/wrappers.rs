use parking_lot::{MappedRwLockReadGuard, RwLockReadGuard};

use account::{Account, StakingContract};
use block::{Block, BlockType, MacroBlock};
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use blockchain_base::Direction;
use database::{Transaction, WriteTransaction};
use genesis::NetworkInfo;
use hash::{Blake2bHash, Hash};
use primitives::policy;
use primitives::slot::ValidatorSlots;
use transaction::Transaction as BlockchainTransaction;
use utils::merkle;

use crate::blockchain_state::BlockchainState;
use crate::Blockchain;

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

    /// Returns the current state.
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

    /// Fetches a given block, by its hash.
    fn get_block(&self, hash: &Blake2bHash, include_body: bool) -> Option<Block> {
        self.chain_store.get_block(hash, include_body, None)
    }

    /// Fetches a given number of blocks, starting at a specific block (by its hash).
    fn get_blocks(
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
            // It might be that we synced this epoch via macro block sync.
            // Therefore, we check this first.
            let macro_block = policy::election_block_of(batch_or_epoch_index);
            if let Some(macro_block_info) =
                self.chain_store
                    .get_chain_info_at(macro_block, false, txn_option)
            {
                if let Some(txs) = self
                    .chain_store
                    .get_epoch_transactions(&macro_block_info.head.hash(), txn_option)
                {
                    return Some(txs);
                }
            }
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
        let hashes: Vec<Blake2bHash> = self
            .get_epoch_transactions(epoch, txn_option)?
            .iter()
            .map(|tx| tx.hash())
            .collect(); // BlockchainTransaction::hash does *not* work here.

        Some(merkle::compute_root_from_hashes::<Blake2bHash>(&hashes))
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
}
