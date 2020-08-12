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

/// Wrapper functions
impl Blockchain {
    pub fn head(&self) -> MappedRwLockReadGuard<Block> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| &s.main_chain.head)
    }

    pub fn macro_head(&self) -> MappedRwLockReadGuard<MacroBlock> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| s.macro_info.head.unwrap_macro_ref())
    }

    pub fn election_head(&self) -> MappedRwLockReadGuard<MacroBlock> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| &s.election_head)
    }
    pub fn head_hash(&self) -> Blake2bHash {
        self.state.read().head_hash.clone()
    }

    pub fn macro_head_hash(&self) -> Blake2bHash {
        self.state.read().macro_head_hash.clone()
    }

    pub fn election_head_hash(&self) -> Blake2bHash {
        self.state.read().election_head_hash.clone()
    }

    pub fn block_number(&self) -> u32 {
        self.state.read_recursive().main_chain.head.block_number()
    }

    pub fn view_number(&self) -> u32 {
        self.state.read_recursive().main_chain.head.view_number()
    }

    pub fn next_view_number(&self) -> u32 {
        self.state
            .read_recursive()
            .main_chain
            .head
            .next_view_number()
    }

    pub fn current_validators(&self) -> MappedRwLockReadGuard<ValidatorSlots> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| s.current_validators().unwrap())
    }

    pub fn last_validators(&self) -> MappedRwLockReadGuard<ValidatorSlots> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| s.last_validators().unwrap())
    }

    pub fn state(&self) -> RwLockReadGuard<BlockchainState> {
        self.state.read()
    }

    pub fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool {
        match self.chain_store.get_chain_info(hash, false, None) {
            Some(chain_info) => include_forks || chain_info.on_main_chain,
            None => false,
        }
    }

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

    // gets transactions for either an epoch or a batch.
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

        // We need to make sure that we have all micro blocks.
        if blocks.len() as u32
            != if for_batch {
                policy::BATCH_LENGTH
            } else {
                policy::EPOCH_LENGTH
            } - 2
        {
            debug!(
                "Exptected {} blocks, but get_blocks returned {}",
                if for_batch {
                    policy::BATCH_LENGTH
                } else {
                    policy::EPOCH_LENGTH
                } - 2,
                blocks.len()
            );
            for block in &blocks {
                debug!("Returned block {} - {}", block.block_number(), block.hash());
            }
            return None;
        }

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

    pub fn get_batch_transactions(
        &self,
        batch: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<BlockchainTransaction>> {
        self.get_transactions(batch, true, txn_option)
    }

    pub fn get_epoch_transactions(
        &self,
        epoch: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<BlockchainTransaction>> {
        self.get_transactions(epoch, false, txn_option)
    }

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
