use std::cmp;
use std::collections::HashMap;
use std::collections::HashSet;
use std::convert::TryInto;
use std::sync::Arc;

use failure::Fail;
use parking_lot::{MappedRwLockReadGuard, Mutex, RwLock, RwLockReadGuard};
use parking_lot::MutexGuard;

use account::{Account, AccountError, Inherent, InherentType};
use accounts::Accounts;
use block::{Block, BlockError, BlockHeader, BlockType, MacroBlock, MicroBlock, ValidatorSlots};
use block::ForkProof;
use block::ViewChange;
use blockchain_base::{AbstractBlockchain, BlockchainError, Direction};
use bls::bls12_381::{PublicKey, Signature};
use database::{Environment, ReadTransaction, Transaction, WriteTransaction};
use fixed_unsigned::RoundHalfUp;
use fixed_unsigned::types::{FixedScale10, FixedScale26, FixedUnsigned10, FixedUnsigned26};
use hash::{Blake2bHash, Hash};
use keys::Address;
use network_primitives::networks::NetworkInfo;
use network_primitives::time::NetworkTime;
use primitives::networks::NetworkId;
use primitives::policy;
use primitives::slot::Slot;
use transaction::{TransactionReceipt, TransactionsProof};
use tree_primitives::accounts_proof::AccountsProof;
use tree_primitives::accounts_tree_chunk::AccountsTreeChunk;
use utils::observer::{Listener, ListenerHandle, Notifier};

use crate::chain_info::ChainInfo;
use crate::chain_store::ChainStore;
use crate::transaction_cache::TransactionCache;

pub type PushResult = blockchain_base::PushResult;
pub type PushError = blockchain_base::PushError<BlockError>;
pub type BlockchainEvent = blockchain_base::BlockchainEvent<Block>;

pub struct Blockchain<'env> {
    pub(crate) env: &'env Environment,
    pub network_id: NetworkId,
    network_time: Arc<NetworkTime>,
    pub notifier: RwLock<Notifier<'env, BlockchainEvent>>,
    pub(crate) chain_store: ChainStore<'env>,
    pub(crate) state: RwLock<BlockchainState<'env>>,
    pub push_lock: Mutex<()>, // TODO: Not very nice to have this public
}

pub struct BlockchainState<'env> {
    accounts: Accounts<'env>,
    transaction_cache: TransactionCache,
    pub(crate) main_chain: ChainInfo,
    head_hash: Blake2bHash,
    last_macro_block: MacroBlock
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

    pub fn new(env: &'env Environment, network_id: NetworkId, network_time: Arc<NetworkTime>) -> Result<Self, BlockchainError> {
        let chain_store = ChainStore::new(env);
        Ok(match chain_store.get_head(None) {
            Some(head_hash) => Blockchain::load(env, network_time, network_id, chain_store, head_hash)?,
            None => Blockchain::init(env, network_time, network_id, chain_store)?
        })
    }

    fn load(env: &'env Environment, network_time: Arc<NetworkTime>, network_id: NetworkId, chain_store: ChainStore<'env>, head_hash: Blake2bHash) -> Result<Self, BlockchainError> {
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

        // Initialize TransactionCache.
        let mut transaction_cache = TransactionCache::new();
        let blocks = chain_store.get_blocks_backward(&head_hash, transaction_cache.missing_blocks() - 1, true, None);
        for block in blocks.iter().rev() {
            if let Block::Micro(ref micro_block) = block {
                transaction_cache.push_block(micro_block);
            }
        }
        if let Block::Micro(ref micro_block) = main_chain.head {
            transaction_cache.push_block(&micro_block);
        }
        assert_eq!(transaction_cache.missing_blocks(), policy::TRANSACTION_VALIDITY_WINDOW.saturating_sub(main_chain.head.block_number()));

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
                last_macro_block: unimplemented!()
            }),
            push_lock: Mutex::new(()),
        })
    }

    fn init(env: &'env Environment, network_time: Arc<NetworkTime>, network_id: NetworkId, chain_store: ChainStore<'env>) -> Result<Self, BlockchainError> {
        // Initialize chain & accounts with genesis block.
        let network_info = NetworkInfo::from_network_id(network_id);
        let genesis_block = network_info.genesis_block::<Block>();
        let genesis_macro_block = (match genesis_block { Block::Macro(ref macro_block) => Some(macro_block), _ => None }).unwrap();
        let main_chain = ChainInfo::initial(genesis_block.clone());
        let head_hash = network_info.genesis_hash().clone();

        // Initialize accounts.
        let accounts = Accounts::new(env);
        let mut txn = WriteTransaction::new(env);
        accounts.init(&mut txn, network_id);

        // Commit genesis block to accounts.
        // TODO

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
                last_macro_block: genesis_macro_block.clone()
            }),
            push_lock: Mutex::new(()),
        })
    }

    pub fn macro_head_hash(&self) -> Blake2bHash { unimplemented!(); }

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
            warn!("Rejecting block - wrong block number ({:?})", block.ty());
            return Err(PushError::InvalidSuccessor)
        }

        if let Block::Micro(ref micro_block) = block {
            // Check if a view change occurred - if so, validate the proof
            if prev_info.head.view_number() != block.view_number() {
                match micro_block.justification.view_change_proof {
                    None => return Err(PushError::InvalidSuccessor),
                    Some(ref view_change_proof) => {
                        let view_change = ViewChange { block_number: block.block_number(), new_view_number: block.view_number()};
                        let idx_to_key_closure = |idx: u16| -> ValidatorSlots { self.get_current_validator_by_idx(idx).unwrap() };
                        if !view_change_proof.into_trusted(idx_to_key_closure).verify(&view_change, policy::TWO_THIRD_VALIDATORS) {
                            return Err(PushError::InvalidSuccessor)
                        }
                    }
                }
            }

            // Check if the block was produced (and signed) by the intended producer
            let intended_slot_owner = self.get_next_block_producer(block.view_number()).1.public_key;
            if !intended_slot_owner.verify(&micro_block.serialize_without_signature(), &micro_block.justification.signature) {
                warn!("Rejecting block - not a valid successor");
                return Err(PushError::InvalidSuccessor);
            }

            // Validate slash inherents
            for fork_proof in &micro_block.extrinsics.as_ref().unwrap().fork_proofs {
                match self.get_block_producer_at(fork_proof.header1.block_number, fork_proof.header1.view_number) {
                    None => {
                        warn!("Rejecting block - Bad fork proof: Unknown block owner");
                        return Err(PushError::InvalidSuccessor)
                    },
                    Some((idx, slot)) => {
                        if !slot.public_key.verify(&fork_proof.header1, &fork_proof.justification1) ||
                                !slot.public_key.verify(&fork_proof.header2, &fork_proof.justification2) {
                            warn!("Rejecting block - Bad fork proof: invalid owner signature");
                            return Err(PushError::InvalidSuccessor)
                        }
                    }
                }
            }
        }

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
                        return Err(PushError::InvalidSuccessor)
                    },
                    Some(ref justification) => {
                        let idx_to_key_closure = |idx: u16| -> ValidatorSlots { self.get_current_validator_by_idx(idx).unwrap() };
                        if !justification.into_trusted(idx_to_key_closure).verify(macro_block.hash(), policy::TWO_THIRD_VALIDATORS) {
                            warn!("Rejecting block - macro block not sufficiently signed");
                            return Err(PushError::InvalidSuccessor)
                        }
                    }
                }
            },
            Block::Micro(ref micro_block) => {
                if let Err(push_result) = self.push_verify_dry(&block) {
                    return Err(push_result);
                }
            }
        }

        let prev_info = self.chain_store.get_chain_info(&block.parent_hash(), false, None).unwrap();
        let chain_info = prev_info.next(block);

        if chain_info.head.parent_hash() == &self.state.read().main_chain.head.hash() {
            return self.extend(chain_info.head.hash(), chain_info, prev_info);
        }

        if chain_info.head.view_number() > self.state.read().main_chain.head.view_number() {
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

        if let Block::Micro(ref micro_block) = chain_info.head {
            let state = self.state.read();

            // Check transactions against TransactionCache to prevent replay.
            if state.transaction_cache.contains_any(&micro_block) {
                warn!("Rejecting block - transaction already included");
                txn.abort();
                return Err(PushError::DuplicateTransaction);
            }

            // Commit block to AccountsTree.
            if let Err(e) = self.commit_accounts(&state.accounts, &mut txn, &chain_info.head) {
                warn!("Rejecting block - commit failed: {:?}", e);
                txn.abort();
                #[cfg(feature = "metrics")]
                    self.metrics.note_invalid_block();
                return Err(e);
            }
        }

        chain_info.on_main_chain = true;
        prev_info.main_chain_successor = Some(chain_info.head.hash());

        self.chain_store.put_chain_info(&mut txn, &block_hash, &chain_info, true);
        self.chain_store.put_chain_info(&mut txn, &chain_info.head.parent_hash(), &prev_info, false);
        self.chain_store.set_head(&mut txn, &block_hash);

        if let Block::Micro(ref micro_block) = chain_info.head {
            // Acquire write lock.
            let mut state = self.state.write();
            state.transaction_cache.push_block(&micro_block);
            state.main_chain = chain_info;
            state.head_hash = block_hash;
            txn.commit();
        }

        // Give up write lock before notifying.
        let event;
        {
            let state = self.state.read();
            event = BlockchainEvent::Extended(state.head_hash.clone());
        }
        self.notifier.read().notify(event);

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
        {
            let state = self.state.read();

            cache_txn = state.transaction_cache.clone();
            // XXX Get rid of the .clone() here.
            current = (state.head_hash.clone(), state.main_chain.clone());

            while current.0 != ancestor.0 {
                match current.1.head {
                    Block::Macro(_) => unreachable!(),
                    Block::Micro(ref micro_block) => {
                        self.revert_accounts(&state.accounts, &mut write_txn, &micro_block);

                        cache_txn.revert_block(micro_block);

                        let prev_hash = micro_block.header.parent_hash.clone();
                        let prev_info = self.chain_store
                            .get_chain_info(&prev_hash, true, Some(&read_txn))
                            .expect("Corrupted store: Failed to find main chain predecessor while rebranching");

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
                match block {
                    Block::Macro(_) => unreachable!(),
                    Block::Micro(ref micro_block) => cache_txn.prepend_block(micro_block)
                }
            }
            assert_eq!(cache_txn.missing_blocks(), policy::TRANSACTION_VALIDITY_WINDOW.saturating_sub(ancestor.1.head.block_number()));

            // Check each fork block against TransactionCache & commit to AccountsTree.
            let mut fork_iter = fork_chain.iter().rev();
            while let Some(fork_block) = fork_iter.next() {
                match fork_block.1.head {
                    Block::Macro(_) => unreachable!(),
                    Block::Micro(ref micro_block) => {

                        let result = if !cache_txn.contains_any(micro_block) {
                            self.commit_accounts(&state.accounts, &mut write_txn, &fork_block.1.head)
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

                        cache_txn.push_block(micro_block);
                    }
                }
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
            write_txn.commit();
            state.transaction_cache = cache_txn;

            state.main_chain = fork_chain[0].1.clone();
            state.head_hash = fork_chain[0].0.clone();
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

        Ok(PushResult::Rebranched)
    }

    fn commit_accounts(&self, accounts: &Accounts, txn: &mut WriteTransaction, block: &Block) -> Result<(), PushError> {

        match block {
            Block::Macro(ref macro_block) => {
                // TODO commit transaction fees, commit slash distribution

                // Commit block to AccountsTree.
                let receipts = accounts.commit(txn, &vec![], &vec![], macro_block.header.block_number);
                if let Err(e) = receipts {
                    return Err(PushError::AccountsError(e));
                }
            },
            Block::Micro(ref micro_block) => {
                let extrinsics = micro_block.extrinsics.as_ref().unwrap();

                let fork_proofs = match self.create_fork_proofs(&micro_block) {
                    Err(e) => return Err(e),
                    Ok(fork_proofs) => fork_proofs
                };

                // Commit block to AccountsTree.
                let receipts = accounts.commit(txn, &extrinsics.transactions, &fork_proofs, micro_block.header.block_number);
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

    fn revert_accounts(&self, accounts: &Accounts, txn: &mut WriteTransaction, micro_block: &MicroBlock) -> Result<(), PushError> {
        let extrinsics = micro_block.extrinsics.as_ref().unwrap();

        assert_eq!(micro_block.header.state_root, accounts.hash(Some(&txn)),
                   "Failed to revert - inconsistent state");

        let fork_proofs = match self.create_fork_proofs(&micro_block) {
            Err(e) => return Err(e),
            Ok(fork_proofs) => fork_proofs
        };

        if let Err(e) = accounts.revert(txn, &extrinsics.transactions, &fork_proofs, micro_block.header.block_number, &extrinsics.receipts) {
            panic!("Failed to revert - {}", e);
        }

        Ok(())
    }

    pub fn create_fork_proofs(&self, micro_block: &MicroBlock) -> Result<Vec<Inherent>, PushError> {
        let mut fork_proofs = Vec::new();
        for fork_proof in &micro_block.extrinsics.as_ref().unwrap().fork_proofs {
            let producer_opt = self.get_block_producer_at(fork_proof.header1.block_number, fork_proof.header1.view_number);
            if producer_opt.is_none() {
                return Err(PushError::InvalidSuccessor);
            }

            fork_proofs.push(Inherent {
                ty: InherentType::Slash,
                target: producer_opt.unwrap().1.slashing_address.clone(),
                value: self.last_macro_block().extrinsics.as_ref().unwrap().slashing_amount.try_into().unwrap(),
                data: vec![]
            });
        }
        Ok(fork_proofs)
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

    pub fn height(&self) -> u32 {
        self.state.read().main_chain.head.block_number()
    }

    pub fn head(&self) -> MappedRwLockReadGuard<Block> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| &s.main_chain.head)
    }

    pub fn last_macro_block(&self) -> MappedRwLockReadGuard<MacroBlock> {
        let guard = self.state.read();
        RwLockReadGuard::map(guard, |s| &s.last_macro_block)
    }

    pub fn get_next_block_producer(&self, view_number: u32) -> (u16, Slot) {
        unimplemented!();
    }

    pub fn get_next_validator_list(&self) -> Vec<Slot> {
        unimplemented!()
    }

    pub fn get_next_validator_set(&self) -> Vec<ValidatorSlots> {
        unimplemented!()
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

    pub fn get_current_validator_by_idx(&self, validator_idx: u16) -> Option<ValidatorSlots> { unimplemented!() }

    // Checks if a block number is within the range of the current epoch
    pub fn is_in_current_epoch(&self, block_number: u32) -> bool {
        return block_number >= self.state.read().last_macro_block.header.block_number && block_number < self.state.read().last_macro_block.header.block_number + policy::EPOCH_LENGTH;
    }

    pub fn get_block_producer_at(&self, block_number: u32, view_number: u32) -> Option<(/*Index in slot list*/ u16, &Slot)> { unimplemented!() }

    pub fn state(&self) -> RwLockReadGuard<BlockchainState<'env>> {
        self.state.read()
    }

    pub fn fork_proofs_to_inherents(&self, fork_proofs: &Vec<ForkProof>) -> Vec<Inherent> { unimplemented!() }

    pub fn finalized_last_epoch(&self) -> Vec<Inherent> { unimplemented!() }
}

impl<'env> AbstractBlockchain<'env> for Blockchain<'env> {
    type Block = Block;

    fn new(env: &'env Environment, network_id: NetworkId, network_time: Arc<NetworkTime>) -> Result<Self, BlockchainError> {
        unimplemented!()
    }

    fn network_id(&self) -> NetworkId {
        unimplemented!()
    }

    fn head_block(&self) -> Self::Block {
        unimplemented!()
    }

    fn head_hash(&self) -> Blake2bHash {
        unimplemented!()
    }

    fn head_height(&self) -> u32 {
        unimplemented!()
    }

    fn get_block(&self, hash: &Blake2bHash, include_body: bool) -> Option<Self::Block> {
        unimplemented!()
    }

    fn get_block_at(&self, height: u32, include_body: bool) -> Option<Self::Block> {
        unimplemented!()
    }

    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        unimplemented!()
    }

    fn get_blocks(&self, start_block_hash: &Blake2bHash, count: u32, include_body: bool, direction: Direction) -> Vec<Self::Block> {
        unimplemented!()
    }

    fn push(&self, block: Self::Block) -> Result<PushResult, PushError> {
        unimplemented!()
    }

    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool {
        unimplemented!()
    }

    fn get_accounts_proof(&self, block_hash: &Blake2bHash, addresses: &[Address]) -> Option<AccountsProof> {
        unimplemented!()
    }

    fn get_transactions_proof(&self, block_hash: &Blake2bHash, addresses: &HashSet<Address>) -> Option<TransactionsProof> {
        unimplemented!()
    }

    fn get_transaction_receipts_by_address(&self, address: &Address, sender_limit: usize, recipient_limit: usize) -> Vec<TransactionReceipt> {
        unimplemented!()
    }

    fn register_listener<T: Listener<BlockchainEvent> + 'env>(&self, listener: T) -> ListenerHandle {
        unimplemented!()
    }

    fn lock(&self) -> MutexGuard<()> {
        unimplemented!()
    }

    fn get_account(&self, address: &Address) -> Account {
        unimplemented!()
    }

    fn contains_tx_in_validity_window(&self, tx_hash: &Blake2bHash) -> bool {
        unimplemented!()
    }

    fn head_hash_from_store(&self, txn: &ReadTransaction) -> Option<Blake2bHash> {
        unimplemented!()
    }

    fn get_accounts_chunk(&self, prefix: &str, size: usize, txn_option: Option<&Transaction>) -> Option<AccountsTreeChunk> {
        unimplemented!()
    }
}
