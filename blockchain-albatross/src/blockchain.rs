use std::cmp;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::convert::TryInto;
use std::iter::FromIterator;
use std::sync::Arc;

use parking_lot::{MappedRwLockReadGuard, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockUpgradableReadGuard};

use account::{Account, Inherent, InherentType};
use account::inherent::AccountInherentInteraction;
use accounts::Accounts;
use beserial::Serialize;
use block::{Block, BlockError, BlockType, MacroBlock, MacroHeader, MicroBlock, ForkProof, ViewChanges, ViewChangeProof};
use blockchain_base::{AbstractBlockchain, BlockchainError, Direction};
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use collections::bitset::BitSet;
use database::{Environment, ReadTransaction, Transaction, WriteTransaction};
use hash::Blake2bHash;
use keys::Address;
use network_primitives::networks::NetworkInfo;
use network_primitives::time::NetworkTime;
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use primitives::policy;
use primitives::validators::{Slot, Slots, Validator, Validators};
use transaction::{TransactionReceipt, TransactionsProof};
use tree_primitives::accounts_proof::AccountsProof;
use tree_primitives::accounts_tree_chunk::AccountsTreeChunk;
use utils::observer::{Listener, ListenerHandle, Notifier};

use crate::chain_info::ChainInfo;
use crate::chain_store::ChainStore;
use crate::reward_registry::{EpochStateError, SlashRegistry, SlashedSlots};
use crate::transaction_cache::TransactionCache;

pub type PushResult = blockchain_base::PushResult;
pub type PushError = blockchain_base::PushError<BlockError>;
pub type BlockchainEvent = blockchain_base::BlockchainEvent<Block>;

pub struct Blockchain<'env> {
    pub(crate) env: &'env Environment,
    pub network_id: NetworkId,
    // TODO network_time: Arc<NetworkTime>,
    pub notifier: RwLock<Notifier<'env, BlockchainEvent>>,
    pub(crate) chain_store: Arc<ChainStore<'env>>,
    pub(crate) state: RwLock<BlockchainState<'env>>,
    pub push_lock: Mutex<()>, // TODO: Not very nice to have this public

    #[cfg(feature = "metrics")]
    metrics: BlockchainMetrics,
}

pub struct BlockchainState<'env> {
    accounts: Accounts<'env>,
    transaction_cache: TransactionCache,
    pub(crate) reward_registry: SlashRegistry<'env>,

    pub(crate) main_chain: ChainInfo,
    head_hash: Blake2bHash,

    macro_head: MacroBlock,
    macro_head_hash: Blake2bHash,

    current_validators: Option<Validators>,
    last_validators: Option<Validators>,
    current_slots: Option<Slots>,
    last_slots: Option<Slots>,
}

impl<'env> BlockchainState<'env> {
    pub fn accounts(&self) -> &Accounts<'env> {
        &self.accounts
    }

    pub fn transaction_cache(&self) -> &TransactionCache {
        &self.transaction_cache
    }
}

impl<'env> Blockchain<'env> {
    pub fn new(env: &'env Environment, network_id: NetworkId) -> Result<Self, BlockchainError> {
        let chain_store = Arc::new(ChainStore::new(env));
        Ok(match chain_store.get_head(None) {
            Some(head_hash) => Blockchain::load(env, network_id, chain_store, head_hash)?,
            None => Blockchain::init(env, network_id, chain_store)?
        })
    }

    fn load(env: &'env Environment, network_id: NetworkId, chain_store: Arc<ChainStore<'env>>, head_hash: Blake2bHash) -> Result<Self, BlockchainError> {
        // Check that the correct genesis block is stored.
        let network_info = NetworkInfo::from_network_id(network_id);
        let genesis_info = chain_store.get_chain_info(network_info.genesis_hash(), false, None);
        if !genesis_info.map(|i| i.on_main_chain).unwrap_or(false) {
            return Err(BlockchainError::InvalidGenesisBlock)
        }

        // Load main chain from store.
        let main_chain = chain_store
            .get_chain_info(&head_hash, true, None)
            .ok_or(BlockchainError::FailedLoadingMainChain)?;

        // Check that chain/accounts state is consistent.
        let accounts = Accounts::new(env);
        if main_chain.head.state_root() != &accounts.hash(None) {
            return Err(BlockchainError::InconsistentState);
        }

        // Load macro chain from store.
        let macro_chain_info = chain_store
            .get_chain_info_at(policy::last_macro_block(main_chain.head.block_number()), true, None)
            .ok_or(BlockchainError::FailedLoadingMainChain)?;
        let macro_head = match macro_chain_info.head {
            Block::Macro(macro_head) => macro_head,
            Block::Micro(_) => return Err(BlockchainError::InconsistentState),
        };
        let macro_head_hash = macro_head.hash();

        // Initialize TransactionCache.
        let mut transaction_cache = TransactionCache::new();
        let blocks = chain_store.get_blocks_backward(&head_hash, transaction_cache.missing_blocks() - 1, true, None);
        for block in blocks.iter().rev() {
            transaction_cache.push_block(block);
        }
        transaction_cache.push_block(&main_chain.head);
        assert_eq!(transaction_cache.missing_blocks(), policy::TRANSACTION_VALIDITY_WINDOW.saturating_sub(main_chain.head.block_number() + 1));

        // Initialize SlashRegistry.
        let slash_registry = SlashRegistry::new(env, Arc::clone(&chain_store));

        // Current slots and validators
        let (current_slots, current_validators) = Self::slots_and_validators_from_block(&macro_head);

        // Get last slots and validators
        let prev_block = chain_store.get_block(&macro_head.header.parent_macro_hash, true, None);
        let (last_slots, last_validators) = match prev_block {
            Some(Block::Macro(prev_macro_block)) => Self::slots_and_validators_from_block(&prev_macro_block),
            None => (Slots::new(vec![], Coin::ZERO), Validators::new()),
            _ => return Err(BlockchainError::InconsistentState),
        };

        Ok(Blockchain {
            env,
            network_id,
            //network_time,
            notifier: RwLock::new(Notifier::new()),
            chain_store,
            state: RwLock::new(BlockchainState {
                accounts,
                transaction_cache,
                reward_registry: slash_registry,
                main_chain,
                head_hash,
                macro_head,
                macro_head_hash,
                current_slots: Some(current_slots),
                current_validators: Some(current_validators),
                last_slots: Some(last_slots),
                last_validators: Some(last_validators),
            }),
            push_lock: Mutex::new(()),

            #[cfg(feature = "metrics")]
            metrics: BlockchainMetrics::default()
        })
    }

    fn init(env: &'env Environment, network_id: NetworkId, chain_store: Arc<ChainStore<'env>>) -> Result<Self, BlockchainError> {
        // Initialize chain & accounts with genesis block.
        let network_info = NetworkInfo::from_network_id(network_id);
        let genesis_block = network_info.genesis_block::<Block>();
        let genesis_macro_block = (match genesis_block { Block::Macro(ref macro_block) => Some(macro_block), _ => None }).unwrap();
        let main_chain = ChainInfo::initial(genesis_block.clone());
        let head_hash = network_info.genesis_hash().clone();

        // Initialize accounts.
        let accounts = Accounts::new(env);
        let mut txn = WriteTransaction::new(env);
        accounts.init(&mut txn, network_info.genesis_accounts());

        // Commit genesis block to accounts.
        // XXX Don't distribute any reward for the genesis block, so there is nothing to commit.

        // Store genesis block.
        chain_store.put_chain_info(&mut txn, &head_hash, &main_chain, true);
        chain_store.set_head(&mut txn, &head_hash);
        txn.commit();

        // Initialize empty TransactionCache.
        let transaction_cache = TransactionCache::new();

        // Initialize SlashRegistry.
        let slash_registry = SlashRegistry::new(env, Arc::clone(&chain_store));

        // current slots and validators
        let (current_slots, current_validators) = Self::slots_and_validators_from_block(&genesis_macro_block);
        let (last_slots, last_validators) = (Slots::new(vec![], Coin::ZERO), Validators::new());

        Ok(Blockchain {
            env,
            network_id,
            //network_time,
            notifier: RwLock::new(Notifier::new()),
            chain_store,
            state: RwLock::new(BlockchainState {
                accounts,
                transaction_cache,
                reward_registry: slash_registry,
                main_chain,
                head_hash: head_hash.clone(),
                macro_head: genesis_macro_block.clone(),
                macro_head_hash: head_hash,
                current_slots: Some(current_slots.clone()),
                current_validators: Some(current_validators.clone()),
                last_slots: Some(last_slots),
                last_validators: Some(last_validators),
            }),
            push_lock: Mutex::new(()),

            #[cfg(feature = "metrics")]
            metrics: BlockchainMetrics::default()
        })
    }

    // TODO: Replace by proper conversion traits
    fn slots_and_validators_from_block(block: &MacroBlock) -> (Slots, Validators) {
        let slots: Slots = block.clone().try_into().unwrap();
        let validators: Validators = slots.clone().into();
        (slots, validators)
    }

    pub fn verify_macro_block_header(&self, header: &MacroHeader, _view_change_proof: Option<&ViewChangeProof>) -> Result<(), PushError> {
        // Check if the block's immediate predecessor is part of the chain.
        let prev_info_opt = self.chain_store.get_chain_info(&header.parent_hash, false, None);
        if prev_info_opt.is_none() {
            warn!("Rejecting block - unknown predecessor");
            return Err(PushError::Orphan);
        }

        // Check that the block is a valid successor of its predecessor.
        let prev_info = prev_info_opt.unwrap();

        if self.get_next_block_type(Some(prev_info.head.block_number())) != BlockType::Macro {
            warn!("Rejecting block - not expecting a macro block");
            return Err(PushError::InvalidSuccessor);
        }

        // Check the block number
        if prev_info.head.block_number() + 1 != header.block_number {
            warn!("Rejecting block - wrong block number ({:?})", header.block_number);
            return Err(PushError::InvalidSuccessor)
        }

        // TODO: Check validators

        // TODO: Check view number (ViewChangeProof in PbftProposal)

        // TODO: Check parent macro hash

        // TODO: Check seed (need current block producer)

        // TODO: Check timestamp

        Ok(())
    }

    /// Verification required for macro block proposals and micro blocks
    pub fn push_verify_dry(&self, block: &Block) -> Result<(), PushError> {
        // Check (sort of) intrinsic block invariants.
        if let Err(e) = block.verify(self.network_id) {
            warn!("Rejecting block - verification failed ({:?})", e);
            return Err(PushError::InvalidBlock(e));
        }

        // Check if the block's immediate predecessor is part of the chain.
        let prev_info_opt = self.chain_store.get_chain_info(&block.parent_hash(), false, None);
        if prev_info_opt.is_none() {
            warn!("Rejecting block - unknown predecessor");
            return Err(PushError::Orphan);
        }

        // Check that the block is a valid successor of its predecessor.
        let prev_info = prev_info_opt.unwrap();

        if self.get_next_block_type(Some(prev_info.head.block_number())) != block.ty() {
            warn!("Rejecting block - wrong block type ({:?})", block.ty());
            return Err(PushError::InvalidSuccessor);
        }

        // Check the block number
        if prev_info.head.block_number() + 1 != block.block_number() {
            warn!("Rejecting block - wrong block number ({:?})", block.block_number());
            return Err(PushError::InvalidSuccessor)
        }

        if let Block::Micro(ref micro_block) = block {
            // Check if a view change occurred - if so, validate the proof
            if prev_info.head.view_number() != block.view_number() {
                match micro_block.justification.view_change_proof {
                    None => return Err(PushError::InvalidBlock(BlockError::NoViewChangeProof)),
                    Some(ref view_change_proof) => view_change_proof.verify(
                        &micro_block.view_change().unwrap(),
                        &self.current_validators(),
                        policy::TWO_THIRD_VALIDATORS)
                        .map_err(|_| BlockError::InvalidJustification)?,
                }
            } else if micro_block.justification.view_change_proof.is_some() {
                return Err(PushError::InvalidBlock(BlockError::InvalidJustification));
            }

            // XXX We might want to pass this as argument to this method
            let read_txn = ReadTransaction::new(self.env);

            // Check if the block was produced (and signed) by the intended producer
            // Public keys are checked when staking, so this should never fail.
            let intended_slot_owner = if let Some((_, slot)) = self
                .get_block_producer_at(block.block_number(), block.view_number(), Some(&read_txn)) {
                slot.public_key.uncompress_unchecked().clone()
            } else {
                return Err(PushError::InvalidSuccessor);
            };

            let justification = micro_block.justification.signature.uncompress()
                .map_err(|_| PushError::InvalidBlock(BlockError::InvalidJustification))?;
            if !intended_slot_owner.verify(&micro_block.header, &justification) {
                warn!("Rejecting block - not a valid successor");
                return Err(PushError::InvalidSuccessor);
            }

            // Validate slash inherents
            for fork_proof in &micro_block.extrinsics.as_ref().unwrap().fork_proofs {
                match self.get_block_producer_at(fork_proof.header1.block_number, fork_proof.header1.view_number, Some(&read_txn)) {
                    None => {
                        warn!("Rejecting block - Bad fork proof: Unknown block owner");
                        return Err(PushError::InvalidSuccessor)
                    },
                    Some((_idx, slot)) => {
                        if !fork_proof.verify(&slot.public_key.uncompress_unchecked()).is_ok() {
                            warn!("Rejecting block - Bad fork proof: invalid owner signature");
                            return Err(PushError::InvalidSuccessor)
                        }
                    }
                }
            }
        }

        // TODO: Macro block (proposal) checking. Also generate and validate MacroExtrinsics if None.

        return Ok(());
    }

    pub fn push(&self, block: Block) -> Result<PushResult, PushError> {
        // Only one push operation at a time.
        let _lock = self.push_lock.lock();

        // Check if we already know this block.
        let hash: Blake2bHash = block.hash();
        if self.chain_store.get_chain_info(&hash, false, None).is_some() {
            return Ok(PushResult::Known);
        }

        match block {
            Block::Macro(ref macro_block) => {
                match macro_block.justification {
                    None => {
                        warn!("Rejecting block - macro block without justification");
                        return Err(PushError::InvalidBlock(BlockError::NoJustification))
                    },
                    Some(ref justification) => {
                        trace!("checking justification for macro block: {:#?}", &macro_block.justification);
                        justification.verify(
                            macro_block.hash(),
                            &self.current_validators(),
                            policy::TWO_THIRD_VALIDATORS)
                            .map_err(|_| BlockError::InvalidJustification)?
                    },
                }
            },
            Block::Micro(ref _micro_block) => self.push_verify_dry(&block)?,
        }

        let prev_info = self.chain_store.get_chain_info(&block.parent_hash(), false, None).unwrap();
        let chain_info = prev_info.next(block);

        if *chain_info.head.parent_hash() == self.head_hash() {
            return self.extend(chain_info.head.hash(), chain_info, prev_info);
        }

        let is_better_chain = Ordering::Equal
            .then_with(|| chain_info.head.view_number().cmp(&self.view_number()))
            .then_with(|| chain_info.head.block_number().cmp(&self.block_number()))
            .eq(&Ordering::Greater);
        if is_better_chain {
            return self.rebranch(chain_info.head.hash(), chain_info);
        }

        // Otherwise, we are creating/extending a fork. Store ChainInfo.
        debug!("Creating/extending fork with block {}, block number #{}, view number {}", chain_info.head.hash(), chain_info.head.block_number(), chain_info.head.view_number());
        let mut txn = WriteTransaction::new(self.env);
        self.chain_store.put_chain_info(&mut txn, &chain_info.head.hash(), &chain_info, true);
        txn.commit();

        Ok(PushResult::Forked)
    }

    fn extend(&self, block_hash: Blake2bHash, mut chain_info: ChainInfo, mut prev_info: ChainInfo) -> Result<PushResult, PushError> {
        let mut txn = WriteTransaction::new(self.env);
        let state = self.state.upgradable_read();

        // Check transactions against TransactionCache to prevent replay.
        // XXX This is technically unnecessary for macro blocks, but it doesn't hurt either.
        if state.transaction_cache.contains_any(&chain_info.head) {
            warn!("Rejecting block - transaction already included");
            txn.abort();
            return Err(PushError::DuplicateTransaction);
        }

        if let Err(e) = state.reward_registry.commit_block(&mut txn, &chain_info.head, state.current_slots.as_ref().expect("Current slots missing while rebranching")) {
            warn!("Rejecting block - slash commit failed: {:?}", e);
            return Err(PushError::InvalidSuccessor);
        }

        // Commit block to AccountsTree.
        if let Err(e) = self.commit_accounts(&state, &mut txn, &chain_info.head) {
            warn!("Rejecting block - commit failed: {:?}", e);
            txn.abort();
            #[cfg(feature = "metrics")]
                self.metrics.note_invalid_block();
            return Err(e);
        }

        chain_info.on_main_chain = true;
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        self.chain_store.put_chain_info(&mut txn, &block_hash, &chain_info, true);
        self.chain_store.put_chain_info(&mut txn, &chain_info.head.parent_hash(), &prev_info, false);
        self.chain_store.set_head(&mut txn, &block_hash);

        // Acquire write lock & commit changes.
        let mut state = RwLockUpgradableReadGuard::upgrade(state);
        state.transaction_cache.push_block(&chain_info.head);

        if let Block::Macro(ref macro_block) = chain_info.head {
            state.macro_head = macro_block.clone();
            state.macro_head_hash = block_hash.clone();

            let slots = state.current_slots.take().unwrap();
            let validators = state.current_validators.take().unwrap();
            state.last_slots.replace(slots);
            state.last_validators.replace(validators);

            let (slot, validators) = Self::slots_and_validators_from_block(&macro_block);
            state.current_slots.replace(slot);
            state.current_validators.replace(validators);
        }

        let block_type = chain_info.head.ty();

        state.main_chain = chain_info;
        state.head_hash = block_hash.clone();
        txn.commit();

        // Give up lock before notifying.
        drop(state);

        let event = BlockchainEvent::Extended(block_hash);
        self.notifier.read().notify(event);

        if block_type == BlockType::Macro {
            self.notifier.read().notify(BlockchainEvent::Finalized);
        }

        Ok(PushResult::Extended)
    }

    fn rebranch(&self, block_hash: Blake2bHash, chain_info: ChainInfo) -> Result<PushResult, PushError> {
        debug!("Rebranching to fork {}, height #{}, view number {}", block_hash, chain_info.head.block_number(), chain_info.head.view_number());

        // Find the common ancestor between our current main chain and the fork chain.
        // Walk up the fork chain until we find a block that is part of the main chain.
        // Store the chain along the way.
        let read_txn = ReadTransaction::new(self.env);

        let mut fork_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];
        let mut current: (Blake2bHash, ChainInfo) = (block_hash, chain_info);
        while !current.1.on_main_chain {
            // Macro blocks are final
            assert_eq!(current.1.head.ty(), BlockType::Micro, "Trying to rebranch across macro block");

            let prev_hash = current.1.head.parent_hash().clone();
            let prev_info = self.chain_store
                .get_chain_info(&prev_hash, true, Some(&read_txn))
                .expect("Corrupted store: Failed to find fork predecessor while rebranching");

            fork_chain.push(current);
            current = (prev_hash, prev_info);
        }

        debug!("Found common ancestor {} at height #{}, {} blocks up", current.0, current.1.head.block_number(), fork_chain.len());

        // Revert AccountsTree & TransactionCache to the common ancestor state.
        let mut revert_chain: Vec<(Blake2bHash, ChainInfo)> = vec![];
        let mut ancestor = current;

        let mut write_txn = WriteTransaction::new(self.env);
        let mut cache_txn;

        let state = self.state.upgradable_read();
        cache_txn = state.transaction_cache.clone();
        // XXX Get rid of the .clone() here.
        current = (state.head_hash.clone(), state.main_chain.clone());

        while current.0 != ancestor.0 {
            match current.1.head {
                Block::Macro(_) => unreachable!(),
                Block::Micro(ref micro_block) => {
                    let prev_hash = micro_block.header.parent_hash.clone();
                    let prev_info = self.chain_store
                        .get_chain_info(&prev_hash, true, Some(&read_txn))
                        .expect("Corrupted store: Failed to find main chain predecessor while rebranching");

                    self.revert_accounts(&state.accounts, &mut write_txn, &micro_block, prev_info.head.view_number())?;

                    let slots = state.current_slots.as_ref().expect("Current slots missing while rebranching");
                    state.reward_registry.revert_block(&mut write_txn, &current.1.head, slots).unwrap();

                    cache_txn.revert_block(&current.1.head);

                    assert_eq!(prev_info.head.state_root(), &state.accounts.hash(Some(&write_txn)),
                               "Failed to revert main chain while rebranching - inconsistent state");

                    revert_chain.push(current);
                    current = (prev_hash, prev_info);
                }
            }
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
            cache_txn.prepend_block(block)
        }
        assert_eq!(cache_txn.missing_blocks(), policy::TRANSACTION_VALIDITY_WINDOW.saturating_sub(ancestor.1.head.block_number() + 1));

        // Check each fork block against TransactionCache & commit to AccountsTree and SlashRegistry.
        let mut fork_iter = fork_chain.iter().rev();
        while let Some(fork_block) = fork_iter.next() {
            match fork_block.1.head {
                Block::Macro(_) => unreachable!(),
                Block::Micro(ref micro_block) => {
                    let result = if !cache_txn.contains_any(&fork_block.1.head) {
                        state.reward_registry.commit_block(&mut write_txn, &fork_block.1.head, state.current_slots.as_ref().expect("Current slots missing while rebranching"))
                            .map_err(|_| PushError::InvalidBlock(BlockError::InvalidSlash))
                            .and_then(|_| self.commit_accounts(&state, &mut write_txn, &fork_block.1.head))

                    } else {
                        Err(PushError::DuplicateTransaction)
                    };

                    if let Err(e) = result {
                        warn!("Failed to apply fork block while rebranching - {:?}", e);
                        write_txn.abort();

                        // Delete invalid fork blocks from store.
                        let mut write_txn = WriteTransaction::new(self.env);
                        for block in vec![fork_block].into_iter().chain(fork_iter) {
                            self.chain_store.remove_chain_info(&mut write_txn, &block.0, micro_block.header.block_number)
                        }
                        write_txn.commit();

                        return Err(PushError::InvalidFork);
                    }

                    cache_txn.push_block(&fork_block.1.head);
                }
            }
        }

        // Fork looks good.
        // Acquire write lock.
        let mut state = RwLockUpgradableReadGuard::upgrade(state);

        // Unset onMainChain flag / mainChainSuccessor on the current main chain up to (excluding) the common ancestor.
        for reverted_block in revert_chain.iter_mut() {
            reverted_block.1.on_main_chain = false;
            reverted_block.1.main_chain_successor = None;
            self.chain_store.put_chain_info(&mut write_txn, &reverted_block.0, &reverted_block.1, false);
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
        }

        // Commit transaction & update head.
        self.chain_store.set_head(&mut write_txn, &fork_chain[0].0);
        state.transaction_cache = cache_txn;
        state.main_chain = fork_chain[0].1.clone();
        state.head_hash = fork_chain[0].0.clone();
        write_txn.commit();

        // Give up lock before notifying.
        drop(state);

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

        Ok(PushResult::Rebranched)
    }

    fn commit_accounts(&self, state: &BlockchainState, txn: &mut WriteTransaction, block: &Block) -> Result<(), PushError> {
        let accounts = &state.accounts;

        match block {
            Block::Macro(ref macro_block) => {
                let inherents = self.finalize_last_epoch(state);

                // Commit block to AccountsTree.
                let receipts = accounts.commit(txn, &vec![], &inherents, macro_block.header.block_number);
                if let Err(e) = receipts {
                    return Err(PushError::AccountsError(e));
                }
            },
            Block::Micro(ref micro_block) => {
                let extrinsics = micro_block.extrinsics.as_ref().unwrap();
                let view_changes = ViewChanges::new(micro_block.header.block_number, self.view_number(), micro_block.header.view_number);
                let inherents = self.create_slash_inherents(&extrinsics.fork_proofs, &view_changes, Some(txn));

                // Commit block to AccountsTree.
                let receipts = accounts.commit(txn, &extrinsics.transactions, &inherents, micro_block.header.block_number);
                if let Err(e) = receipts {
                    return Err(PushError::AccountsError(e));
                }

                // Verify receipts.
                if extrinsics.receipts != receipts.unwrap() {
                    return Err(PushError::InvalidBlock(BlockError::InvalidReceipt));
                }
            }
        }

        // Verify accounts hash.
        if block.state_root() != &accounts.hash(Some(&txn)) {
            return Err(PushError::InvalidBlock(BlockError::AccountsHashMismatch));
        }

        Ok(())
    }

    fn revert_accounts(&self, accounts: &Accounts, txn: &mut WriteTransaction, micro_block: &MicroBlock, prev_view_number: u32) -> Result<(), PushError> {
        assert_eq!(micro_block.header.state_root, accounts.hash(Some(&txn)),
                   "Failed to revert - inconsistent state");

        let extrinsics = micro_block.extrinsics.as_ref().unwrap();
        let view_changes = ViewChanges::new(micro_block.header.block_number, prev_view_number, micro_block.header.view_number);
        let inherents = self.create_slash_inherents(&extrinsics.fork_proofs, &view_changes, Some(txn));

        if let Err(e) = accounts.revert(txn, &extrinsics.transactions, &inherents, micro_block.header.block_number, &extrinsics.receipts) {
            panic!("Failed to revert - {}", e);
        }

        Ok(())
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

    pub fn height(&self) -> u32 {
        self.block_number()
    }

    pub fn block_number(&self) -> u32 {
        self.state.read().main_chain.head.block_number()
    }

    pub fn view_number(&self) -> u32 {
        self.state.read().main_chain.head.view_number()
    }

    pub fn head(&self) -> MappedRwLockReadGuard<Block> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| &s.main_chain.head)
    }

    pub fn head_hash(&self) -> Blake2bHash {
        self.state.read().head_hash.clone()
    }

    pub fn macro_head(&self) -> MappedRwLockReadGuard<MacroBlock> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| &s.macro_head)
    }

    pub fn macro_head_hash(&self) -> Blake2bHash {
        self.state.read().macro_head_hash.clone()
    }

    pub fn get_next_block_producer(&self, view_number: u32, txn_option: Option<&Transaction>) -> (u16, Slot) {
        let block_number =self.height() + 1;
        self.get_block_producer_at(block_number, view_number, txn_option)
            .expect("Can't get block producer for next block")
    }

    pub fn next_slots(&self) -> Slots {
        let validator_registry = NetworkInfo::from_network_id(self.network_id).validator_registry_address().expect("No ValidatorRegistry");
        let staking_account = self.state.read().accounts().get(validator_registry, None);
        if let Account::Staking(ref staking_contract) = staking_account {
            return staking_contract.select_validators(self.state.read().main_chain.head.seed(), policy::ACTIVE_VALIDATORS, policy::MAX_CONSIDERED as usize);
        }
        panic!("Account at validator registry address is not the stacking contract!");
    }

    pub fn next_validators(&self) -> Validators {
        self.next_slots().into()
    }

    pub fn get_next_block_type(&self, last_number: Option<u32>) -> BlockType {
        let last_block_number = match last_number {
            None => self.head().block_number(),
            Some(n) => n

        };
        match (last_block_number + 1) % policy::EPOCH_LENGTH {
            0 => BlockType::Macro,
            _ => BlockType::Micro
        }
    }

    pub fn get_current_validator_by_idx(&self, validator_idx: u16) -> Option<Validator> {
        self.current_validators().get(validator_idx as usize).map(Validator::clone)
    }

    // Checks if a block number is within the range of the current epoch
    pub fn is_in_current_epoch(&self, block_number: u32) -> bool {
        let state = self.state.read();
        self.is_in_current_epoch_locked(block_number, &state)
    }

    fn is_in_current_epoch_locked(&self, block_number: u32, state: &RwLockReadGuard<BlockchainState>) -> bool {
        let macro_block_number = state.macro_head.header.block_number;
        block_number > macro_block_number && block_number <= macro_block_number + policy::EPOCH_LENGTH
    }

    fn is_in_previous_epoch_locked(&self, block_number: u32, state: &RwLockReadGuard<BlockchainState>) -> bool {
        let macro_block_number = state.macro_head.header.block_number;
        macro_block_number > policy::EPOCH_LENGTH && block_number >= macro_block_number - policy::EPOCH_LENGTH && block_number < macro_block_number
    }

    pub fn get_block_producer_at(&self, block_number: u32, view_number: u32, txn_option: Option<&Transaction>) -> Option<(/*Index in slot list*/ u16, Slot)> {
        let state = self.state.read();

        let read_txn;
        let txn = if let Some(txn) = txn_option {
            txn
        }
        else {
            read_txn = ReadTransaction::new(self.env);
            &read_txn
        };

        let slots_owned;
        let slots;
        if self.is_in_current_epoch_locked(block_number, &state) {
            slots = state.current_slots.as_ref().expect("Missing current epoch's slots");
        } else if self.is_in_previous_epoch_locked(block_number, &state) {
            slots = state.last_slots.as_ref().expect("Missing previous epoch's slots");
        } else {
            let macro_block = self.chain_store
                .get_block_at(policy::macro_block_before(block_number), Some(&txn))
                .expect("Failed to determine slots - preceding macro block not found")
                .unwrap_macro();

            // Get slots of epoch
            slots_owned = macro_block.try_into().unwrap();
            slots = &slots_owned;
        }

        self.state.read().reward_registry.slot_owner(block_number, view_number, slots, Some(&txn))
    }

    pub fn state(&self) -> RwLockReadGuard<BlockchainState<'env>> {
        self.state.read()
    }

    pub fn create_slash_inherents(&self, fork_proofs: &Vec<ForkProof>, view_changes: &Option<ViewChanges>, txn_option: Option<&Transaction>) -> Vec<Inherent> {
        let mut inherents = vec![];
        for fork_proof in fork_proofs {
            inherents.push(self.inherent_from_fork_proof(fork_proof, txn_option));
        }
        if let Some(view_changes) = view_changes {
            inherents.append(&mut self.inherents_from_view_changes(view_changes, txn_option));
        }
        inherents
    }

    /// Expects a *verified* proof!
    pub fn inherent_from_fork_proof(&self, fork_proof: &ForkProof, txn_option: Option<&Transaction>) -> Inherent {
        let producer = self.get_block_producer_at(fork_proof.header1.block_number, fork_proof.header1.view_number, txn_option)
            .expect("Failed to create inherent from fork proof - could not get block producer");
        let validator_registry = NetworkInfo::from_network_id(self.network_id).validator_registry_address().expect("No ValidatorRegistry");
        Inherent {
            ty: InherentType::Slash,
            target: validator_registry.clone(),
            value: self.slash_fine_at(fork_proof.header1.block_number),
            data: producer.1.staker_address.serialize_to_vec(),
        }
    }

    /// Expects a *verified* proof!
    pub fn inherents_from_view_changes(&self, view_changes: &ViewChanges, txn_option: Option<&Transaction>) -> Vec<Inherent> {
        let validator_registry = NetworkInfo::from_network_id(self.network_id).validator_registry_address().expect("No ValidatorRegistry");

        (view_changes.first_view_number .. view_changes.last_view_number).map(|view_number| {
            let producer = self.get_block_producer_at(view_changes.block_number, view_number, txn_option)
                .expect("Failed to create inherent from fork proof - could not get block producer");
            Inherent {
                ty: InherentType::Slash,
                target: validator_registry.clone(),
                value: self.slash_fine_at(view_changes.block_number),
                data: producer.1.staker_address.serialize_to_vec(),
            }
        }).collect::<Vec<Inherent>>()
    }

    fn slash_fine_at(&self, block_number: u32) -> Coin {
        let current_epoch = policy::epoch_at(self.block_number() + 1);
        let slash_epoch = policy::epoch_at(block_number);
        if slash_epoch == current_epoch {
            self.current_slots().slash_fine()
        } else if slash_epoch + 1 == current_epoch {
            self.last_slots().slash_fine()
        } else {
            // TODO Error handling
            panic!("Tried to retrieve slash fine outside of reporting window");
        }
    }

    pub fn current_slots(&self) -> MappedRwLockReadGuard<Slots> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| s.current_slots.as_ref().unwrap())
    }

    pub fn last_slots(&self) -> MappedRwLockReadGuard<Slots> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| s.last_slots.as_ref().unwrap())
    }

    pub fn current_slashed_set(&self) -> BitSet {
        let s = self.state.read();
        s.reward_registry.slashed_set(policy::epoch_at(self.block_number()), None)
    }

    pub fn last_slashed_set(&self) -> BitSet {
        let s = self.state.read();
        s.reward_registry.slashed_set(policy::epoch_at(self.block_number()) - 1, None)
    }

    // Get slash set of epoch at specific block number
    // Returns slash set before applying block with that block_number (TODO Tests)
    pub fn slashed_set_at(&self, epoch_number: u32, block_number: u32) -> Result<BitSet, EpochStateError> {
        let s = self.state.read();
        s.reward_registry.slashed_set_at(epoch_number, block_number, None)
    }

    pub fn current_validators(&self) -> MappedRwLockReadGuard<Validators> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| s.current_validators.as_ref().unwrap())
    }

    pub fn last_validators(&self) -> MappedRwLockReadGuard<Validators> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| s.last_validators.as_ref().unwrap())
    }

    pub fn finalize_last_epoch(&self, state: &BlockchainState) -> Vec<Inherent> {
        let mut inherents = Vec::new();

        let epoch = policy::epoch_at(state.main_chain.head.block_number()) - 1;

        // Special case for first epoch: Epoch 0 is finalized by definition.
        if epoch == 0 {
            return vec![];
        }

        /*
        let block = self.get_block_at(policy::macro_block_of(epoch), true);
        let macro_block= if let Some(Block::Macro(block)) = block {
            block
        } else {
            // TODO: Proper error handling.
            panic!("Macro block to finalize is missing");
        };
        */

        // Find slots that are eligible for rewards.
        // TODO: Proper error handling.
        let slots = state.last_slots.clone().expect("Slots for last epoch are missing");
        let slashed_set = state.reward_registry.slashed_set(epoch, None);
        let reward_eligible = Vec::from_iter(SlashedSlots::new(&slots, &slashed_set).enabled().cloned());

        let reward_pot: Coin = state.reward_registry.previous_reward_pot();
        let num_eligible = reward_eligible.len() as u64;

        let initial_reward = reward_pot / num_eligible;
        let mut remainder = reward_pot % num_eligible;

        for slot in reward_eligible.iter() {
            let reward = initial_reward;

            let inherent = Inherent {
                ty: InherentType::Reward,
                target: slot.reward_address_opt.clone().unwrap_or(slot.staker_address.clone()),
                value: reward,
                data: vec![],
            };

            // Test whether account will accept inherent.
            let account = state.accounts.get(&inherent.target, None);
            if account.check_inherent(&inherent).is_err() {
                remainder += reward;
            } else {
                inherents.push(inherent);
            }
        }

        // Distribute over accepting slots.
        let accepting_slots = inherents.len() as u64;
        let reward = remainder / accepting_slots;
        let remainder = u64::from(remainder % accepting_slots);

        for (i, inherent) in inherents.iter_mut().enumerate() {
            let mut additional_reward = reward;
            // Distribute remaining reward across the largest stakers.
            if (i as u64) < remainder {
                additional_reward += Coin::from_u64_unchecked(1);
            }

            inherent.value += additional_reward;
        }

        inherents
    }
}

impl<'env> AbstractBlockchain<'env> for Blockchain<'env> {
    type Block = Block;

    fn new(env: &'env Environment, network_id: NetworkId, _network_time: Arc<NetworkTime>) -> Result<Self, BlockchainError> {
        Blockchain::new(env, network_id)
    }

    #[cfg(feature = "metrics")]
    fn metrics(&self) -> &BlockchainMetrics {
        &self.metrics
    }

    fn network_id(&self) -> NetworkId {
        self.network_id
    }

    fn head_block(&self) -> MappedRwLockReadGuard<Self::Block> {
        self.head()
    }

    fn head_hash(&self) -> Blake2bHash {
        self.head_hash()
    }

    fn head_height(&self) -> u32 {
        self.block_number()
    }

    fn get_block(&self, hash: &Blake2bHash, include_body: bool) -> Option<Self::Block> {
        self.get_block(hash, false, include_body)
    }

    fn get_block_at(&self, height: u32, include_body: bool) -> Option<Self::Block> {
        self.get_block_at(height, include_body)
    }

    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        let mut locators: Vec<Blake2bHash> = Vec::with_capacity(max_count);
        let mut hash = self.head_hash();

        // push top ten hashes
        locators.push(hash.clone());
        for _ in 0..cmp::min(10, self.height()) {
            let block = self.chain_store.get_block(&hash, false, None);
            match block {
                Some(block) => {
                    hash = block.header().parent_hash().clone();
                    locators.push(hash.clone());
                },
                None => break,
            }
        }

        let mut step = 2;
        let mut height = self.height().saturating_sub(10 + step);
        let mut opt_block = self.chain_store.get_block_at(height, None);
        while let Some(block) = opt_block {
            locators.push(block.header().hash());

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

            opt_block = self.chain_store.get_block_at(height, None);
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

    fn get_blocks(&self, start_block_hash: &Blake2bHash, count: u32, include_body: bool, direction: Direction) -> Vec<Self::Block> {
        self.get_blocks(start_block_hash, count, include_body, direction)
    }

    fn push(&self, block: Self::Block) -> Result<PushResult, PushError> {
        self.push(block)
    }

    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool {
        self.contains(hash, include_forks)
    }

    #[allow(unused_variables)]
    fn get_accounts_proof(&self, block_hash: &Blake2bHash, addresses: &[Address]) -> Option<AccountsProof> {
        unimplemented!()
    }

    #[allow(unused_variables)]
    fn get_transactions_proof(&self, block_hash: &Blake2bHash, addresses: &HashSet<Address>) -> Option<TransactionsProof> {
        unimplemented!()
    }

    #[allow(unused_variables)]
    fn get_transaction_receipts_by_address(&self, address: &Address, sender_limit: usize, recipient_limit: usize) -> Vec<TransactionReceipt> {
        unimplemented!()
    }

    fn register_listener<T: Listener<BlockchainEvent> + 'env>(&self, listener: T) -> ListenerHandle {
        self.notifier.write().register(listener)
    }

    fn lock(&self) -> MutexGuard<()> {
        self.push_lock.lock()
    }

    fn get_account(&self, address: &Address) -> Account {
        self.state.read().accounts.get(address, None)
    }

    fn contains_tx_in_validity_window(&self, tx_hash: &Blake2bHash) -> bool {
        self.state.read().transaction_cache.contains(tx_hash)
    }

    #[allow(unused_variables)]
    fn head_hash_from_store(&self, txn: &ReadTransaction) -> Option<Blake2bHash> {
        unimplemented!()
    }

    fn get_accounts_chunk(&self, prefix: &str, size: usize, txn_option: Option<&Transaction>) -> Option<AccountsTreeChunk> {
        self.state.read().accounts.get_chunk(prefix, size, txn_option)
    }
}
