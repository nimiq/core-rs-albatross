use std::convert::TryInto;

use parking_lot::{MappedRwLockReadGuard, RwLockReadGuard};

use account::Account;
use block::{Block, BlockType, MacroBlock};
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use blockchain_base::Direction;
use database::{ReadTransaction, Transaction, WriteTransaction};
use genesis::NetworkInfo;
use hash::{Blake2bHash, Hash};
use primitives::policy;
use primitives::slot::{Slot, Slots, ValidatorSlots};
use transaction::Transaction as BlockchainTransaction;
use utils::merkle;
use vrf::VrfSeed;

use crate::blockchain_state::BlockchainState;
use crate::slots::{get_slot_at, SlashedSet};
use crate::Blockchain;

// Simple stuff
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

    pub fn get_slots_for_epoch(&self, epoch: u32) -> Option<Slots> {
        let state = self.state.read();
        let current_epoch = policy::epoch_at(state.main_chain.head.block_number());

        let slots = if epoch == current_epoch {
            state.current_slots.as_ref()?.clone()
        } else if epoch == current_epoch - 1 {
            state.previous_slots.as_ref()?.clone()
        } else {
            let macro_block = self
                .chain_store
                .get_block_at(policy::election_block_of(epoch), true, None)?
                .unwrap_macro();
            macro_block.try_into().unwrap()
        };

        Some(slots)
    }

    pub fn get_validators_for_epoch(&self, epoch: u32) -> Option<ValidatorSlots> {
        if let Some(slots) = self.get_slots_for_epoch(epoch) {
            Some(slots.into())
        } else {
            None
        }
    }

    pub fn next_slots(&self, seed: &VrfSeed, txn_option: Option<&Transaction>) -> Slots {
        let validator_registry = NetworkInfo::from_network_id(self.network_id)
            .validator_registry_address()
            .expect("No ValidatorRegistry");
        let staking_account = self
            .state
            .read()
            .accounts()
            .get(validator_registry, txn_option);
        if let Account::Staking(ref staking_contract) = staking_account {
            return staking_contract.select_validators(seed);
        }
        panic!("Account at validator registry address is not the stacking contract!");
    }

    pub fn next_validators(&self, seed: &VrfSeed, txn: Option<&Transaction>) -> ValidatorSlots {
        self.next_slots(seed, txn).into()
    }

    pub fn get_slot_at(
        &self,
        block_number: u32,
        view_number: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<(Slot, u16)> {
        let state = self.state.read_recursive();

        let read_txn;
        let txn = if let Some(txn) = txn_option {
            txn
        } else {
            read_txn = ReadTransaction::new(&self.env);
            &read_txn
        };

        // Gets slots collection from either the cached ones, or from the election block.
        // Note: We need to handle the case where `state.block_number()` is at a macro block (so `state.current_slots`
        // was already updated by it, pushing this epoch's slots to `state.previous_slots` and deleting previous'
        // epoch's slots).
        let slots_owned;
        let slots = if policy::epoch_at(state.block_number()) == policy::epoch_at(block_number) {
            if policy::is_election_block_at(state.block_number()) {
                state.previous_slots.as_ref().unwrap_or_else(|| {
                    panic!(
                        "Missing epoch's slots for block {}.{}",
                        block_number, view_number
                    )
                })
            } else {
                state
                    .current_slots
                    .as_ref()
                    .expect("Missing current epoch's slots")
            }
        } else if !policy::is_election_block_at(state.block_number())
            && policy::epoch_at(state.block_number()) == policy::epoch_at(block_number) + 1
        {
            state.previous_slots.as_ref().unwrap_or_else(|| {
                panic!(
                    "Missing previous epoch's slots for block {}.{}",
                    block_number, view_number
                )
            })
        } else {
            let macro_block = self
                .chain_store
                .get_block_at(
                    policy::election_block_before(block_number),
                    true,
                    Some(&txn),
                )?
                .unwrap_macro();

            // Get slots of epoch
            slots_owned = macro_block.try_into().unwrap();
            &slots_owned
        };

        let prev_info = self
            .chain_store
            .get_chain_info_at(block_number - 1, false, Some(&txn))?;

        Some(get_slot_at(block_number, view_number, &prev_info, slots))
    }

    pub fn get_slot_for_next_block(
        &self,
        view_number: u32,
        txn_option: Option<&Transaction>,
    ) -> (Slot, u16) {
        let block_number = self.block_number() + 1;
        self.get_slot_at(block_number, view_number, txn_option)
            .unwrap()
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
            let macro_block = policy::macro_block_of(batch_or_epoch_index);
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

    fn get_macro_locators(&self, max_count: usize, election_blocks_only: bool) -> Vec<Blake2bHash> {
        let mut locators: Vec<Blake2bHash> = Vec::with_capacity(max_count);
        let mut hash = if election_blocks_only {
            self.election_head_hash()
        } else {
            self.macro_head_hash()
        };

        // Push top ten hashes.
        locators.push(hash.clone());
        for _ in 0..10 {
            let block = self.chain_store.get_block(&hash, false, None);
            match block {
                Some(Block::Macro(block)) => {
                    hash = if election_blocks_only {
                        block.header.parent_election_hash
                    } else {
                        block.header.parent_hash
                    };
                    locators.push(hash.clone());
                }
                Some(_) => unreachable!("macro head must be a macro block"),
                None => break,
            }
        }

        let mut step = 2;
        let mut height = if election_blocks_only {
            policy::last_election_block(self.block_number())
                .saturating_sub((10 + step) * policy::EPOCH_LENGTH)
        } else {
            policy::last_macro_block(self.block_number())
                .saturating_sub((10 + step) * policy::BATCH_LENGTH)
        };

        let mut opt_block = self.chain_store.get_block_at(height, false, None);
        while let Some(block) = opt_block {
            assert_eq!(block.ty(), BlockType::Macro);
            locators.push(block.header().hash());

            // Respect max count.
            if locators.len() >= max_count {
                break;
            }

            step *= 2;
            height = if election_blocks_only {
                height.saturating_sub(step * policy::EPOCH_LENGTH)
            } else {
                height.saturating_sub(step * policy::BATCH_LENGTH)
            };
            // 0 or underflow means we need to end the loop
            if height == 0 {
                break;
            }

            opt_block = self.chain_store.get_block_at(height, false, None);
        }

        // Push the genesis block hash.
        let genesis_hash = NetworkInfo::from_network_id(self.network_id).genesis_hash();
        if locators.is_empty() || locators.last().unwrap() != genesis_hash {
            // Respect max count, make space for genesis hash if necessary
            if locators.len() >= max_count {
                locators.pop();
            }
            locators.push(genesis_hash.clone());
        }

        locators
    }

    pub fn get_macro_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        self.get_macro_locators(max_count, false)
    }

    pub fn get_election_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        self.get_macro_locators(max_count, true)
    }

    // TODO: Needs to be for the whole epoch!!!
    pub fn get_history_root(
        &self,
        batch: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Blake2bHash> {
        let hashes: Vec<Blake2bHash> = self
            .get_batch_transactions(batch, txn_option)?
            .iter()
            .map(|tx| tx.hash())
            .collect(); // BlockchainTransaction::hash does *not* work here.

        Some(merkle::compute_root_from_hashes::<Blake2bHash>(&hashes))
    }

    /// Get slashed set at specific block number
    /// Returns slash set before applying block with that block_number (TODO Tests)
    pub fn slashed_set_at(&self, block_number: u32) -> Option<SlashedSet> {
        let prev_info = self
            .chain_store
            .get_chain_info_at(block_number - 1, false, None)?;
        Some(prev_info.slashed_set)
    }

    pub fn write_transaction(&self) -> WriteTransaction {
        WriteTransaction::new(&self.env)
    }
}
