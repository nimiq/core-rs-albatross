use std::cmp;
use std::sync::Arc;

use parking_lot::{MappedRwLockReadGuard, Mutex, RwLock, RwLockReadGuard};

use account::AccountError;
use accounts::Accounts;
use block::{Block, BlockError, Difficulty, Target, TargetCompact};
use block::proof::ChainProof;
use database::{Environment, ReadTransaction, WriteTransaction};
use fixed_unsigned::RoundHalfUp;
use fixed_unsigned::types::{FixedScale10, FixedScale26, FixedUnsigned10, FixedUnsigned26};
use hash::{Blake2bHash, Hash};
use network_primitives::networks::get_network_info;
use network_primitives::time::NetworkTime;
use primitives::networks::NetworkId;
use primitives::policy;
use utils::observer::Notifier;

use crate::blockchain::error::BlockchainError;
use crate::chain_info::ChainInfo;
#[cfg(feature = "metrics")]
use crate::chain_metrics::BlockchainMetrics;
use crate::chain_store::{ChainStore, Direction};
use crate::transaction_cache::TransactionCache;
#[cfg(feature = "transaction-store")]
use crate::transaction_store::TransactionStore;

pub mod transaction_proofs;
pub mod error;

pub struct Blockchain<'env> {
    pub(crate) env: &'env Environment,
    pub network_id: NetworkId,
    network_time: Arc<NetworkTime>,
    pub notifier: RwLock<Notifier<'env, BlockchainEvent>>,
    pub(crate) chain_store: ChainStore<'env>,
    pub(crate) state: RwLock<BlockchainState<'env>>,
    pub push_lock: Mutex<()>, // TODO: Not very nice to have this public

    #[cfg(feature = "metrics")]
    pub metrics: BlockchainMetrics,

    #[cfg(feature = "transaction-store")]
    pub(crate) transaction_store: TransactionStore<'env>,
}

pub struct BlockchainState<'env> {
    accounts: Accounts<'env>,
    transaction_cache: TransactionCache,
    pub(crate) main_chain: ChainInfo,
    head_hash: Blake2bHash,
    pub(crate) chain_proof: Option<ChainProof>,
}

impl<'env> BlockchainState<'env> {
    pub fn accounts(&self) -> &Accounts<'env> {
        &self.accounts
    }

    pub fn transaction_cache(&self) -> &TransactionCache {
        &self.transaction_cache
    }

    pub fn main_chain(&self) -> &ChainInfo {
        &self.main_chain
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushResult {
    Invalid(PushError),
    Orphan,
    Known,
    Extended,
    Rebranched,
    Forked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError {
    InvalidBlock(BlockError),
    InvalidSuccessor,
    DifficultyMismatch,
    DuplicateTransaction,
    AccountsError(AccountError),
    InvalidFork,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockchainEvent {
    Extended(Blake2bHash),
    Rebranched(Vec<(Blake2bHash, Block)>, Vec<(Blake2bHash, Block)>),
}

impl<'env> Blockchain<'env> {
    pub fn new(env: &'env Environment, network_id: NetworkId, network_time: Arc<NetworkTime>) -> Result<Self, BlockchainError> {
        let chain_store = ChainStore::new(env);
        Ok(match chain_store.get_head(None) {
            Some(head_hash) => Blockchain::load(env, network_time, network_id, chain_store, head_hash)?,
            None => Blockchain::init(env, network_time, network_id, chain_store)?
        })
    }

    fn load(env: &'env Environment, network_time: Arc<NetworkTime>, network_id: NetworkId, chain_store: ChainStore<'env>, head_hash: Blake2bHash) -> Result<Self, BlockchainError> {
        // Check that the correct genesis block is stored.
        let network_info = get_network_info(network_id).unwrap();
        let genesis_info = chain_store.get_chain_info(&network_info.genesis_hash, false, None);

        if !genesis_info.map(|i| i.on_main_chain).unwrap_or(false) {
            return Err(BlockchainError::InvalidGenesisBlock)
        }

        // Load main chain from store.
        let main_chain = chain_store
            .get_chain_info(&head_hash, true, None)
            .ok_or(BlockchainError::FailedLoadingMainChain)?;

        // Check that chain/accounts state is consistent.
        let accounts = Accounts::new(env);

        if main_chain.head.header.accounts_hash != accounts.hash(None) {
            return Err(BlockchainError::InconsistentState);
        }

        // Initialize TransactionCache.
        let mut transaction_cache = TransactionCache::new();
        let blocks = chain_store.get_blocks_backward(&head_hash, transaction_cache.missing_blocks() - 1, true, None);
        for block in blocks.iter().rev() {
            transaction_cache.push_block(block);
        }
        transaction_cache.push_block(&main_chain.head);
        assert_eq!(transaction_cache.missing_blocks(), policy::TRANSACTION_VALIDITY_WINDOW.saturating_sub(main_chain.head.header.height));

        Ok(Blockchain {
            env,
            network_id,
            network_time,
            notifier: RwLock::new(Notifier::new()),
            chain_store,
            state: RwLock::new(BlockchainState {
                accounts,
                transaction_cache,
                main_chain,
                head_hash,
                chain_proof: None,
            }),
            push_lock: Mutex::new(()),

            #[cfg(feature = "metrics")]
            metrics: BlockchainMetrics::default(),

            #[cfg(feature = "transaction-store")]
            transaction_store: TransactionStore::new(env),
        })
    }

    fn init(env: &'env Environment, network_time: Arc<NetworkTime>, network_id: NetworkId, chain_store: ChainStore<'env>) -> Result<Self, BlockchainError> {
        // Initialize chain & accounts with genesis block.
        let network_info = get_network_info(network_id).ok_or_else(|| BlockchainError::NoNetwork(network_id))?;
        let main_chain = ChainInfo::initial(network_info.genesis_block.clone());
        let head_hash = network_info.genesis_hash.clone();

        // Initialize accounts.
        let accounts = Accounts::new(env);
        let mut txn = WriteTransaction::new(env);
        accounts.init(&mut txn, network_id);

        // Store genesis block.
        chain_store.put_chain_info(&mut txn, &head_hash, &main_chain, true);
        chain_store.set_head(&mut txn, &head_hash);
        txn.commit();

        // Initialize empty TransactionCache.
        let transaction_cache = TransactionCache::new();

        Ok(Blockchain {
            env,
            network_id,
            network_time,
            notifier: RwLock::new(Notifier::new()),
            chain_store,
            state: RwLock::new(BlockchainState {
                accounts,
                transaction_cache,
                main_chain,
                head_hash,
                chain_proof: None,
            }),
            push_lock: Mutex::new(()),

            #[cfg(feature = "metrics")]
            metrics: BlockchainMetrics::default(),

            #[cfg(feature = "transaction-store")]
            transaction_store: TransactionStore::new(env),
        })
    }

    pub fn push(&self, block: Block) -> PushResult {
        // We expect full blocks (with body).
        assert!(block.body.is_some(), "Block body expected");

        // Check (sort of) intrinsic block invariants.
        let info = get_network_info(self.network_id).unwrap();
        if let Err(e) = block.verify(self.network_time.now(), self.network_id, info.genesis_block.header.hash()) {
            warn!("Rejecting block - verification failed ({:?})", e);
            #[cfg(feature = "metrics")]
            self.metrics.note_invalid_block();
            return PushResult::Invalid(PushError::InvalidBlock(e))
        }

        // Only one push operation at a time.
        let _lock = self.push_lock.lock();

        // Check if we already know this block.
        let hash: Blake2bHash = block.header.hash();
        if self.chain_store.get_chain_info(&hash, false, None).is_some() {
            #[cfg(feature = "metrics")]
            self.metrics.note_known_block();
            return PushResult::Known;
        }

        // Check if the block's immediate predecessor is part of the chain.
        let prev_info_opt = self.chain_store.get_chain_info(&block.header.prev_hash, false, None);
        if prev_info_opt.is_none() {
            warn!("Rejecting block - unknown predecessor");
            #[cfg(feature = "metrics")]
            self.metrics.note_orphan_block();
            return PushResult::Orphan;
        }

        // Check that the block is a valid successor of its predecessor.
        let prev_info = prev_info_opt.unwrap();
        if !block.is_immediate_successor_of(&prev_info.head) {
            warn!("Rejecting block - not a valid successor");
            #[cfg(feature = "metrics")]
            self.metrics.note_invalid_block();
            return PushResult::Invalid(PushError::InvalidSuccessor);
        }

        // Check that the difficulty is correct.
        let next_target = self.get_next_target(Some(&block.header.prev_hash));
        if block.header.n_bits != TargetCompact::from(next_target) {
            warn!("Rejecting block - difficulty mismatch");
            #[cfg(feature = "metrics")]
            self.metrics.note_invalid_block();
            return PushResult::Invalid(PushError::DifficultyMismatch);
        }

        // Block looks good, create ChainInfo.
        let chain_info = prev_info.next(block);

        // Check if the block extends our current main chain.
        if chain_info.head.header.prev_hash == self.state.read().head_hash {
            return self.extend(hash, chain_info, prev_info);
        }

        // Otherwise, check if the new chain is harder than our current main chain.
        if chain_info.total_difficulty > self.state.read().main_chain.total_difficulty {
            // A fork has become the hardest chain, rebranch to it.
            return self.rebranch(hash, chain_info);
        }

        // Otherwise, we are creating/extending a fork. Store ChainInfo.
        debug!("Creating/extending fork with block {}, height #{}, total_difficulty {}", hash, chain_info.head.header.height, chain_info.total_difficulty);
        let mut txn = WriteTransaction::new(self.env);
        self.chain_store.put_chain_info(&mut txn, &hash, &chain_info, true);
        txn.commit();

        #[cfg(feature = "metrics")]
        self.metrics.note_forked_block();
        PushResult::Forked
    }

    fn extend(&self, block_hash: Blake2bHash, mut chain_info: ChainInfo, mut prev_info: ChainInfo) -> PushResult {
        let mut txn = WriteTransaction::new(self.env);
        {
            let state = self.state.read();

            // Check transactions against TransactionCache to prevent replay.
            if state.transaction_cache.contains_any(&chain_info.head) {
                warn!("Rejecting block - transaction already included");
                txn.abort();
                #[cfg(feature = "metrics")]
                self.metrics.note_invalid_block();
                return PushResult::Invalid(PushError::DuplicateTransaction);
            }

            // Commit block to AccountsTree.
            if let Err(e) = state.accounts.commit_block(&mut txn, &chain_info.head) {
                warn!("Rejecting block - commit failed: {}", e);
                txn.abort();
                #[cfg(feature = "metrics")]
                self.metrics.note_invalid_block();
                return PushResult::Invalid(PushError::AccountsError(e));
            }
        }

        chain_info.on_main_chain = true;
        prev_info.main_chain_successor = Some(block_hash.clone());

        self.chain_store.put_chain_info(&mut txn, &block_hash, &chain_info, true);
        self.chain_store.put_chain_info(&mut txn, &chain_info.head.header.prev_hash, &prev_info, false);
        self.chain_store.set_head(&mut txn, &block_hash);

        {
            // Acquire write lock.
            let mut state = self.state.write();

            state.transaction_cache.push_block(&chain_info.head);

            #[cfg(feature = "transaction-store")]
            self.transaction_store.put(&chain_info.head, &mut txn);

            state.main_chain = chain_info;
            state.head_hash = block_hash;

            state.chain_proof = None;

            txn.commit();
        }

        // Give up write lock before notifying.
        let event;
        {
            let state = self.state.read();
            event = BlockchainEvent::Extended(state.head_hash.clone());
        }
        self.notifier.read().notify(event);

        #[cfg(feature = "metrics")]
        self.metrics.note_extended_block();
        PushResult::Extended
    }

    fn rebranch(&self, block_hash: Blake2bHash, chain_info: ChainInfo) -> PushResult {
        debug!("Rebranching to fork {}, height #{}, total_difficulty {}", block_hash, chain_info.head.header.height, chain_info.total_difficulty);

        // Find the common ancestor between our current main chain and the fork chain.
        // Walk up the fork chain until we find a block that is part of the main chain.
        // Store the chain along the way.
        let read_txn = ReadTransaction::new(self.env);

        let mut fork_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];
        let mut current: (Blake2bHash, ChainInfo) = (block_hash, chain_info);
        while !current.1.on_main_chain {
            let prev_hash = current.1.head.header.prev_hash.clone();
            let prev_info = self.chain_store
                .get_chain_info(&prev_hash, true, Some(&read_txn))
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            fork_chain.push(current);
            current = (prev_hash, prev_info);
        }

        debug!("Found common ancestor {} at height #{}, {} blocks up", current.0, current.1.head.header.height, fork_chain.len());

        // Revert AccountsTree & TransactionCache to the common ancestor state.
        let mut revert_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];
        let mut ancestor = current;

        let mut write_txn = WriteTransaction::new(self.env);
        let mut cache_txn;
        {
            let state = self.state.read();

            cache_txn = state.transaction_cache.clone();
            // XXX Get rid of the .clone() here.
            current = (state.head_hash.clone(), state.main_chain.clone());

            while current.0 != ancestor.0 {
                if let Err(e) = state.accounts.revert_block(&mut write_txn, &current.1.head) {
                    panic!("Failed to revert main chain while rebranching - {}", e);
                }

                cache_txn.revert_block(&current.1.head);

                let prev_hash = current.1.head.header.prev_hash.clone();
                let prev_info = self.chain_store
                    .get_chain_info(&prev_hash, true, Some(&read_txn))
                    .expect("Corrupted store: Failed to find main chain predecessor while rebranching");

                assert_eq!(prev_info.head.header.accounts_hash, state.accounts.hash(Some(&write_txn)),
                           "Failed to revert main chain while rebranching - inconsistent state");

                revert_chain.push(current);
                current = (prev_hash, prev_info);
            }

            // Fetch missing blocks for TransactionCache.
            assert!(cache_txn.is_empty() || cache_txn.head_hash() == ancestor.0);
            let start_hash = if cache_txn.is_empty() {
                ancestor.1.main_chain_successor.unwrap()
            } else {
                cache_txn.tail_hash()
            };
            let blocks = self.chain_store.get_blocks_backward(&start_hash, cache_txn.missing_blocks(), true, Some(&read_txn));
            for block in blocks.iter() {
                cache_txn.prepend_block(block);
            }
            assert_eq!(cache_txn.missing_blocks(), policy::TRANSACTION_VALIDITY_WINDOW.saturating_sub(ancestor.1.head.header.height));

            // Check each fork block against TransactionCache & commit to AccountsTree.
            let mut fork_iter = fork_chain.iter().rev();
            while let Some(fork_block) = fork_iter.next() {
                let result = if !cache_txn.contains_any(&fork_block.1.head) {
                    state.accounts.commit_block(&mut write_txn, &fork_block.1.head).map_err(|e| format!("{}", e))
                } else {
                    Err("Transaction already included".to_string())
                };

                if let Err(e) = result {
                    warn!("Failed to apply fork block while rebranching - {}", e);
                    write_txn.abort();

                    // Delete invalid fork blocks from store.
                    let mut write_txn = WriteTransaction::new(self.env);
                    for block in vec![fork_block].into_iter().chain(fork_iter) {
                        self.chain_store.remove_chain_info(&mut write_txn, &block.0, block.1.head.header.height)
                    }
                    write_txn.commit();

                    #[cfg(feature = "metrics")]
                    self.metrics.note_invalid_block();

                    return PushResult::Invalid(PushError::InvalidFork);
                }

                cache_txn.push_block(&fork_block.1.head);
            }
        }

        // Fork looks good.

        {
            // Acquire write lock.
            let mut state = self.state.write();

            // Unset onMainChain flag / mainChainSuccessor on the current main chain up to (excluding) the common ancestor.
            for reverted_block in revert_chain.iter_mut() {
                reverted_block.1.on_main_chain = false;
                reverted_block.1.main_chain_successor = None;
                self.chain_store.put_chain_info(&mut write_txn, &reverted_block.0, &reverted_block.1, false);

                #[cfg(feature = "transaction-store")]
                self.transaction_store.remove(&reverted_block.1.head, &mut write_txn);
            }

            // Update the mainChainSuccessor of the common ancestor block.
            ancestor.1.main_chain_successor = Some(fork_chain.last().unwrap().0.clone());
            self.chain_store.put_chain_info(&mut write_txn, &ancestor.0, &ancestor.1, false);

            // Set onMainChain flag / mainChainSuccessor on the fork.
            for i in (0..fork_chain.len()).rev() {
                let main_chain_successor = if i > 0 {
                    Some(fork_chain[i - 1].0.clone())
                } else {
                    None
                };

                let fork_block = &mut fork_chain[i];
                fork_block.1.on_main_chain = true;
                fork_block.1.main_chain_successor = main_chain_successor;

                // Include the body of the new block (at position 0).
                self.chain_store.put_chain_info(&mut write_txn, &fork_block.0, &fork_block.1, i == 0);

                #[cfg(feature = "transaction-store")]
                self.transaction_store.put(&fork_block.1.head, &mut write_txn);
            }

            // Commit transaction & update head.
            self.chain_store.set_head(&mut write_txn, &fork_chain[0].0);
            write_txn.commit();
            state.transaction_cache = cache_txn;

            state.main_chain = fork_chain[0].1.clone();
            state.head_hash = fork_chain[0].0.clone();

            state.chain_proof = None;
        }

        // Give up write lock before notifying.
        let mut reverted_blocks = Vec::with_capacity(revert_chain.len());
        for (hash, chain_info) in revert_chain.into_iter().rev() {
            reverted_blocks.push((hash, chain_info.head));
        }
        let mut adopted_blocks = Vec::with_capacity(fork_chain.len());
        for (hash, chain_info) in fork_chain.into_iter().rev() {
            adopted_blocks.push((hash, chain_info.head));
        }
        let event = BlockchainEvent::Rebranched(reverted_blocks, adopted_blocks);
        self.notifier.read().notify(event);

        #[cfg(feature = "metrics")]
        self.metrics.note_rebranched_block();
        PushResult::Rebranched
    }

    pub fn get_next_target(&self, head_hash: Option<&Blake2bHash>) -> Target {
        let state = self.state.read();

        let chain_info;
        let head_info = match head_hash {
            Some(hash) => {
                chain_info = self.chain_store
                    .get_chain_info(hash, false, None)
                    .expect("Failed to compute next target - unknown head_hash");
                &chain_info
            }
            None => &state.main_chain
        };

        let tail_height = 1u32.max(head_info.head.header.height.saturating_sub(policy::DIFFICULTY_BLOCK_WINDOW));
        let tail_info;
        if head_info.on_main_chain {
            tail_info = self.chain_store
                .get_chain_info_at(tail_height, false, None)
                .expect("Failed to compute next target - tail block not found");
        } else {
            let mut prev_info;
            let mut prev_hash = head_info.head.header.prev_hash.clone();
            let mut i = 0;
            // XXX Mimic do ... while {} loop control flow.
            while {
                // Loop condition
                prev_info = self.chain_store
                    .get_chain_info(&prev_hash, false, None)
                    .expect("Failed to compute next target - fork predecessor not found");
                prev_hash = prev_info.head.header.prev_hash.clone();

                i < policy::DIFFICULTY_BLOCK_WINDOW && !prev_info.on_main_chain
            } { /* Loop body */ i += 1; }

            if prev_info.on_main_chain && prev_info.head.header.height > tail_height {
                tail_info = self.chain_store
                    .get_chain_info_at(tail_height, false, None)
                    .expect("Failed to compute next target - tail block not found");
            } else {
                tail_info = prev_info;
            }
        }

        let head = &head_info.head.header;
        let tail = &tail_info.head.header;
        assert!(head.height - tail.height == policy::DIFFICULTY_BLOCK_WINDOW
            || (head.height <= policy::DIFFICULTY_BLOCK_WINDOW && tail.height == 1),
            "Failed to compute next target - invalid head/tail block");

        let mut delta_total_difficulty = &head_info.total_difficulty - &tail_info.total_difficulty;
        let mut actual_time = head.timestamp - tail.timestamp;

        // Simulate that the Policy.BLOCK_TIME was achieved for the blocks before the genesis block, i.e. we simulate
        // a sliding window that starts before the genesis block. Assume difficulty = 1 for these blocks.
        if head.height <= policy::DIFFICULTY_BLOCK_WINDOW {
            actual_time += (policy::DIFFICULTY_BLOCK_WINDOW - head.height + 1) * policy::BLOCK_TIME;
            delta_total_difficulty += policy::DIFFICULTY_BLOCK_WINDOW - head.height + 1;
        }

        // Compute the target adjustment factor.
        let expected_time = policy::DIFFICULTY_BLOCK_WINDOW * policy::BLOCK_TIME;
        let mut adjustment = f64::from(actual_time) / f64::from(expected_time);

        // Clamp the adjustment factor to [1 / MAX_ADJUSTMENT_FACTOR, MAX_ADJUSTMENT_FACTOR].
        adjustment = adjustment
            .max(1f64 / policy::DIFFICULTY_MAX_ADJUSTMENT_FACTOR)
            .min(policy::DIFFICULTY_MAX_ADJUSTMENT_FACTOR);

        // Compute the next target.
        let average_difficulty: Difficulty = delta_total_difficulty / policy::DIFFICULTY_BLOCK_WINDOW;
        let average_target: FixedUnsigned10 = &*policy::BLOCK_TARGET_MAX / &average_difficulty.into(); // Do not use Difficulty -> Target conversion here to preserve precision.

        // `bignumber.js` returns 26 decimal places when multiplying a `BigNumber` with 10 decimal
        // places and a `BigNumber` from a `Number` (which then has 16 decimal places).
        let mut next_target = (average_target.into_scale::<FixedScale26, RoundHalfUp>() * FixedUnsigned26::from(adjustment))
            .into_scale::<FixedScale10, RoundHalfUp>();

        // Make sure the target is below or equal the maximum allowed target (difficulty 1).
        // Also enforce a minimum target of 1.
        if next_target > *policy::BLOCK_TARGET_MAX {
            next_target = policy::BLOCK_TARGET_MAX.clone();
        }
        let min_target = FixedUnsigned10::from(1u64);
        if next_target < min_target {
            next_target = min_target;
        }

        // XXX Reduce target precision to nBits precision.
        let n_bits: TargetCompact = Target::from(next_target).into();
        Target::from(n_bits)
    }

    pub fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        // Push top 10 hashes first, then back off exponentially.
        let mut hash = self.head_hash();
        let mut locators = vec![hash.clone()];

        for _ in 0..cmp::min(10, self.height()) {
            let block = self.chain_store.get_block(&hash, false, None);
            match block {
                Some(block) => {
                    hash = block.header.prev_hash.clone();
                    locators.push(hash.clone());
                },
                None => break,
            }
        }

        let mut step = 2;
        let mut height = self.height().saturating_sub(10 + step);
        let mut opt_block = self.chain_store.get_block_at(height);
        while let Some(block) = opt_block {
            locators.push(block.header.hash());

            // Respect max count.
            if locators.len() >= max_count {
                break;
            }

            step *= 2;
            height = match height.checked_sub(step) {
                Some(0) => break, // 0 or underflow means we need to end the loop
                Some(v) => v,
                None => break,
            };

            opt_block = self.chain_store.get_block_at(height);
        }

        // Push the genesis block hash.
        let network_info = get_network_info(self.network_id).unwrap();
        if locators.is_empty() || locators.last().unwrap() != &network_info.genesis_hash {
            // Respect max count, make space for genesis hash if necessary
            if locators.len() >= max_count {
                locators.pop();
            }
            locators.push(network_info.genesis_hash.clone());
        }

        locators
    }

    pub fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool {
        match self.chain_store.get_chain_info(hash, false, None) {
            Some(chain_info) => include_forks || chain_info.on_main_chain,
            None => false
        }
    }

    pub fn get_block_at(&self, height: u32, include_body: bool) -> Option<Block> {
        self.chain_store.get_chain_info_at(height, include_body, None).map(|chain_info| chain_info.head)
    }

    pub fn get_block(&self, hash: &Blake2bHash, include_forks: bool, include_body: bool) -> Option<Block> {
        let chain_info_opt = self.chain_store.get_chain_info(hash, include_body, None);
        if chain_info_opt.is_some() {
            let chain_info = chain_info_opt.unwrap();
            if chain_info.on_main_chain || include_forks {
                return Some(chain_info.head);
            }
        }
        None
    }

    pub fn get_blocks(&self, start_block_hash: &Blake2bHash, count: u32, include_body: bool, direction: Direction) -> Vec<Block> {
        self.chain_store.get_blocks(start_block_hash, count, include_body, direction, None)
    }

    pub fn head_hash(&self) -> Blake2bHash {
        self.state.read().head_hash.clone()
    }

    pub fn head_hash_from_store(&self, txn: &ReadTransaction) -> Option<Blake2bHash> {
        self.chain_store.get_head(Some(txn))
    }

    pub fn height(&self) -> u32 {
        self.state.read().main_chain.head.header.height
    }

    pub fn head(&self) -> MappedRwLockReadGuard<Block> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| &s.main_chain.head)
    }

    pub fn total_work(&self) -> Difficulty {
        self.state.read().main_chain.total_work.clone()
    }

    pub fn state(&self) -> RwLockReadGuard<BlockchainState<'env>> {
        self.state.read()
    }
}
