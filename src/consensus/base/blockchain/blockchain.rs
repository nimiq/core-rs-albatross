use bigdecimal::BigDecimal;
use consensus::base::account::Accounts;
use consensus::base::block::{Block, Target, TargetCompact};
use consensus::base::blockchain::{ChainData, ChainStore, TransactionCache};
use consensus::base::primitive::hash::{Hash, Blake2bHash};
use consensus::networks::{NetworkId, get_network_info};
use consensus::policy;
use network::NetworkTime;
use utils::db::{Environment, ReadTransaction, WriteTransaction};

#[derive(Debug)]
pub struct Blockchain<'env, 'time> {
    env: &'env Environment,
    network_time: &'time NetworkTime,
    pub network_id: NetworkId,

    pub accounts: Accounts<'env>,
    chain_store: ChainStore<'env>,
    transaction_cache: TransactionCache,
    main_chain: ChainData,
    head_hash: Blake2bHash,
}

pub enum PushResult {
    Orphan,
    Invalid,
    Known,
    Extended,
    Rebranched,
    Forked
}

impl<'env, 'time> Blockchain<'env, 'time> {
    pub fn new(env: &'env Environment, network_time: &'time NetworkTime, network_id: NetworkId) -> Self {
        let chain_store = ChainStore::new(env);
        return match chain_store.get_head(None) {
            Some(head_hash) => Blockchain::load(env, network_time, network_id, chain_store, head_hash),
            None => Blockchain::init(env, network_time, network_id, chain_store)
        };
    }

    fn load(env: &'env Environment, network_time: &'time NetworkTime, network_id: NetworkId, chain_store: ChainStore<'env>, head_hash: Blake2bHash) -> Self {
        // Check that the correct genesis block is stored.
        let network_info = get_network_info(network_id).unwrap();
        let genesis_data = chain_store.get_chain_data(&network_info.genesis_hash, false, None);
        assert!(genesis_data.is_some() && genesis_data.unwrap().on_main_chain,
            "Invalid genesis block stored. Reset your consensus database.");

        // Load main chain from store.
        let main_chain = chain_store
            .get_chain_data(&head_hash, true, None)
            .expect("Failed to load main chain. Reset your consensus database.");

        // Check that chain/accounts state is consistent.
        let accounts = Accounts::new(env);
        assert_eq!(main_chain.head.header.accounts_hash, accounts.hash(None),
            "Inconsistent chain/accounts state. Reset your consensus database.");

        // Initialize TransactionCache.
        let mut transaction_cache = TransactionCache::new();
        let blocks = chain_store.get_blocks_backward(&head_hash, transaction_cache.missing_blocks() - 1, true);
        for block in blocks.iter().rev() {
            transaction_cache.push_block(block);
        }
        transaction_cache.push_block(&main_chain.head);
        assert_eq!(transaction_cache.missing_blocks(), policy::TRANSACTION_VALIDITY_WINDOW.saturating_sub(main_chain.head.header.height));

        return Blockchain { env, network_time, network_id, accounts, chain_store, transaction_cache, main_chain, head_hash };
    }

    fn init(env: &'env Environment, network_time: &'time NetworkTime, network_id: NetworkId, chain_store: ChainStore<'env>) -> Self {
        // Initialize chain & accounts with genesis block.
        let network_info = get_network_info(network_id).unwrap();
        let main_chain = ChainData::initial(network_info.genesis_block.clone());
        let head_hash = network_info.genesis_hash.clone();

        // Store genesis block.
        let mut txn = WriteTransaction::new(env);
        chain_store.put_chain_data(&mut txn, &head_hash, &main_chain, true);
        chain_store.set_head(&mut txn, &head_hash);
        // FIXME commit chain & accounts txn together
        txn.commit();

        // TODO Initialize accounts.
        let accounts = Accounts::new(env);

        // Initialize empty TransactionCache.
        let transaction_cache = TransactionCache::new();

        return Blockchain { env, network_time, network_id, accounts, chain_store, transaction_cache, main_chain, head_hash };
    }

    pub fn push(&mut self, block: Block) -> PushResult {
        // Check if we already know this block.
        let hash: Blake2bHash = block.header.hash();
        if self.chain_store.get_chain_data(&hash, false, None).is_some() {
            return PushResult::Known;
        }

        // Check that the block body is present.
        if block.body.is_none() {
            warn!("Rejecting block - body missing");
            return PushResult::Invalid;
        }

        // Check (sort of) intrinsic block invariants.
        if let Err(e) = block.verify(self.network_time.now(), self.network_id) {
            warn!("Rejecting block - verification failed ({:?})", e);
            return PushResult::Invalid;
        }

        // Check if the block's immediate predecessor is part of the chain.
        let prev_data_opt = self.chain_store.get_chain_data(&block.header.prev_hash, false, None);
        if prev_data_opt.is_none() {
            warn!("Rejecting block - unknown predecessor");
            return PushResult::Orphan;
        }

        // Check that the block is a valid successor of its predecessor.
        let prev_data = prev_data_opt.unwrap();
        if !block.is_immediate_successor_of(&prev_data.head) {
            warn!("Rejecting block - not a valid successor");
            return PushResult::Invalid;
        }

        // Check that the difficulty is correct.
        let next_target = self.get_next_target(Some(&block.header.prev_hash));
        if block.header.n_bits != TargetCompact::from(next_target) {
            warn!("Rejecting block - difficulty mismatch");
            return PushResult::Invalid;
        }

        // Block looks good, create ChainData.
        let chain_data = prev_data.next(block);

        // Check if the block extends our current main chain.
        if chain_data.head.header.prev_hash == self.head_hash {
            return match self.extend(hash, chain_data, prev_data) {
                true => PushResult::Extended,
                false => PushResult::Invalid
            };
        }

        // Otherwise, check if the new chain is harder than our current main chain.
        if chain_data.total_difficulty > self.main_chain.total_difficulty {
            // A fork has become the hardest chain, rebranch to it.
            return match self.rebranch(hash, chain_data) {
                true => PushResult::Rebranched,
                false => PushResult::Invalid
            };
        }

        // Otherwise, we are creating/extending a fork. Store chain data.
        debug!("Creating/extending fork with block {}, height #{}, total_difficulty {}", hash, chain_data.head.header.height, chain_data.total_difficulty);
        let mut txn = WriteTransaction::new(self.env);
        self.chain_store.put_chain_data(&mut txn, &hash, &chain_data, true);
        txn.commit();

        return PushResult::Forked;
    }

    fn extend(&mut self, block_hash: Blake2bHash, mut chain_data: ChainData, mut prev_data: ChainData) -> bool {
        // Check transactions against TransactionCache to prevent replay.
        if self.transaction_cache.contains_any(&chain_data.head) {
            warn!("Rejecting block - transaction already included");
            return false;
        }

        // Commit block to AccountsTree.
        let mut txn = WriteTransaction::new(self.env);
        if let Err(e) = self.accounts.commit_block(&mut txn, &chain_data.head) {
            warn!("Rejecting block - commit failed: {}", e);
            txn.abort();
            return false;
        }

        chain_data.on_main_chain = true;
        prev_data.main_chain_successor = Some(block_hash.clone());

        self.chain_store.put_chain_data(&mut txn, &block_hash, &chain_data, true);
        self.chain_store.put_chain_data(&mut txn, &chain_data.head.header.prev_hash, &prev_data, false);
        self.chain_store.set_head(&mut txn, &block_hash);
        txn.commit();

        self.transaction_cache.push_block(&chain_data.head);

        self.main_chain = chain_data;
        self.head_hash = block_hash;

        return true;
    }

    fn rebranch(&mut self, block_hash: Blake2bHash, chain_data: ChainData) -> bool {
        debug!("Rebranching to fork {}, height #{}, total_difficulty {}", block_hash, chain_data.head.header.height, chain_data.total_difficulty);

        // Find the common ancestor between our current main chain and the fork chain.
        // Walk up the fork chain until we find a block that is part of the main chain.
        // Store the chain along the way.
        let read_txn = ReadTransaction::new(self.env);

        let mut fork_chain: Vec<(Blake2bHash, ChainData)> = vec![];
        let mut current: (Blake2bHash, ChainData) = (block_hash, chain_data);
        while !current.1.on_main_chain {
            let prev_hash = current.1.head.header.prev_hash.clone();
            let prev_data = self.chain_store
                .get_chain_data(&prev_hash, true, Some(&read_txn))
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            fork_chain.push(current);
            current = (prev_hash, prev_data);
        }

        debug!("Found common ancestor {} at height #{}, {} blocks up", current.0, current.1.head.header.height, fork_chain.len());

        // Revert AccountsTree & TransactionCache to the common ancestor state.
        let mut write_txn = WriteTransaction::new(self.env);
        let mut cache_txn = self.transaction_cache.clone();

        let mut revert_chain: Vec<(Blake2bHash, ChainData)> = vec![];
        let mut ancestor = current;
        current = (self.head_hash.clone(), self.main_chain.clone());
        while current.0 != ancestor.0 {
            if let Err(e) = self.accounts.revert_block(&mut write_txn, &current.1.head) {
                panic!("Failed to revert main chain while rebranching - {}", e);
            }

            cache_txn.revert_block(&current.1.head);

            let prev_hash = current.1.head.header.prev_hash.clone();
            let prev_data = self.chain_store
                .get_chain_data(&prev_hash, true, Some(&read_txn))
                .expect("Corrupted store: Failed to find main chain predecessor while rebranching");

            assert_eq!(prev_data.head.header.accounts_hash, self.accounts.hash(Some(&write_txn)),
                "Failed to revert main chain while rebranching - inconsistent state");

            revert_chain.push(current);
            current = (prev_hash, prev_data);
        }

        // Fetch missing blocks for TransactionCache.
        assert!(cache_txn.is_empty() || cache_txn.head_hash() == ancestor.0);
        let start_hash = if cache_txn.is_empty() {
            ancestor.1.main_chain_successor.unwrap()
        } else {
            cache_txn.tail_hash()
        };
        let blocks = self.chain_store.get_blocks_backward(&start_hash, cache_txn.missing_blocks(), true);
        for block in blocks.iter() {
            cache_txn.prepend_block(block);
        }
        assert_eq!(cache_txn.missing_blocks(), policy::TRANSACTION_VALIDITY_WINDOW.saturating_sub(ancestor.1.head.header.height));

        // Check each fork block against TransactionCache & commit to AccountsTree.
        for fork_block in fork_chain.iter().rev() {
            if cache_txn.contains_any(&fork_block.1.head) {
                warn!("Failed to apply fork block while rebranching - transaction already included");
                write_txn.abort();
                return false;
            }

            if let Err(e) = self.accounts.commit_block(&mut write_txn, &fork_block.1.head) {
                warn!("Failed to apply fork block while rebranching - {}", e);
                write_txn.abort();
                return false;
            }

            cache_txn.push_block(&fork_block.1.head);
        }

        // Fork looks good.

        // Unset onMainChain flag / mainChainSuccessor on the current main chain up to (excluding) the common ancestor.
        for reverted_block in revert_chain.iter_mut() {
            reverted_block.1.on_main_chain = false;
            reverted_block.1.main_chain_successor = None;
            self.chain_store.put_chain_data(&mut write_txn, &reverted_block.0, &reverted_block.1, false);
        }

        // Update the mainChainSuccessor of the common ancestor block.
        ancestor.1.main_chain_successor = Some(fork_chain.last().unwrap().0.clone());
        self.chain_store.put_chain_data(&mut write_txn, &ancestor.0, &ancestor.1, false);

        // Set onMainChain flag / mainChainSuccessor on the fork.
        for i in (0..fork_chain.len()).rev() {
            let main_chain_successor = match i > 0 {
                true => Some(fork_chain[i - 1].0.clone()),
                false => None
            };

            let fork_block = &mut fork_chain[i];
            fork_block.1.on_main_chain = true;
            fork_block.1.main_chain_successor = main_chain_successor;

            // Include the body of the new block (at position 0).
            self.chain_store.put_chain_data(&mut write_txn, &fork_block.0, &fork_block.1, i == 0);
        }

        // Commit transaction & update head.
        write_txn.commit();
        self.transaction_cache = cache_txn;

        self.main_chain = fork_chain[0].1.clone(); // TODO get rid of the .clone() here
        self.head_hash = fork_chain[0].0.clone();

        return true;
    }

    pub fn get_next_target(&self, head_hash: Option<&Blake2bHash>) -> Target {
        let chain_data;
        let head_data = match head_hash {
            Some(hash) => {
                chain_data = self.chain_store
                    .get_chain_data(hash, false, None)
                    .expect("Failed to compute next target - unknown head_hash");
                &chain_data
            }
            None => &self.main_chain
        };

        let tail_height = 1u32.max(head_data.head.header.height.saturating_sub(policy::DIFFICULTY_BLOCK_WINDOW));
        let tail_data;
        if head_data.on_main_chain {
            tail_data = self.chain_store
                .get_chain_data_at(tail_height, false, None)
                .expect("Failed to compute next target - tail block not found");
        } else {
            let mut prev_data;
            let mut prev_hash = head_data.head.header.prev_hash.clone();
            let mut i = 0;
            // XXX Mimic do ... while {} loop control flow.
            while {
                // Loop condition
                prev_data = self.chain_store
                    .get_chain_data(&prev_hash, false, None)
                    .expect("Failed to compute next target - fork predecessor not found");
                prev_hash = prev_data.head.header.prev_hash.clone();

                i < policy::DIFFICULTY_BLOCK_WINDOW && !prev_data.on_main_chain
            } { /* Loop body */ i += 1; }

            if prev_data.on_main_chain && prev_data.head.header.height > tail_height {
                tail_data = self.chain_store
                    .get_chain_data_at(tail_height, false, None)
                    .expect("Failed to compute next target - tail block not found");
            } else {
                tail_data = prev_data;
            }
        }

        let head = &head_data.head.header;
        let tail = &tail_data.head.header;
        assert!(head.height - tail.height == policy::DIFFICULTY_BLOCK_WINDOW
            || (head.height <= policy::DIFFICULTY_BLOCK_WINDOW && tail.height == 1),
            "Failed to compute next target - invalid head/tail block");

        let mut delta_total_difficulty = &head_data.total_difficulty - &tail_data.total_difficulty;
        let mut actual_time = head.timestamp - tail.timestamp;

        // Simulate that the Policy.BLOCK_TIME was achieved for the blocks before the genesis block, i.e. we simulate
        // a sliding window that starts before the genesis block. Assume difficulty = 1 for these blocks.
        if head.height <= policy::DIFFICULTY_BLOCK_WINDOW {
            actual_time += (policy::DIFFICULTY_BLOCK_WINDOW - head.height + 1) * policy::BLOCK_TIME;
            delta_total_difficulty += BigDecimal::from(policy::DIFFICULTY_BLOCK_WINDOW - head.height + 1).into();
        }

        // Compute the target adjustment factor.
        let expected_time = policy::DIFFICULTY_BLOCK_WINDOW * policy::BLOCK_TIME;
        let mut adjustment = actual_time as f64 / expected_time as f64;

        // Clamp the adjustment factor to [1 / MAX_ADJUSTMENT_FACTOR, MAX_ADJUSTMENT_FACTOR].
        adjustment = adjustment.max(1f64 / policy::DIFFICULTY_MAX_ADJUSTMENT_FACTOR);
        adjustment = adjustment.min(policy::DIFFICULTY_MAX_ADJUSTMENT_FACTOR);

        // Compute the next target.
        let average_difficulty = BigDecimal::from(delta_total_difficulty) / BigDecimal::from(policy::DIFFICULTY_BLOCK_WINDOW);
        let average_target = &*policy::BLOCK_TARGET_MAX / average_difficulty; // Do not use Difficulty -> Target conversion here to preserve precision.
        let mut next_target = average_target * BigDecimal::from(adjustment);

        // Make sure the target is below or equal the maximum allowed target (difficulty 1).
        // Also enforce a minimum target of 1.
        if next_target > *policy::BLOCK_TARGET_MAX {
            next_target = policy::BLOCK_TARGET_MAX.clone();
        }
        let min_target = BigDecimal::from(1);
        if next_target < min_target {
            next_target = min_target;
        }

        // XXX Reduce target precision to nBits precision.
        let n_bits: TargetCompact = Target::from(next_target).into();
        return Target::from(n_bits);
    }

    pub fn head(&self) -> &Block {
        &self.main_chain.head
    }

    pub fn height(&self) -> u32 {
        self.main_chain.head.header.height
    }
}
