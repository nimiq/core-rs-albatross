use std::sync::Arc;

use parking_lot::{MappedRwLockReadGuard, Mutex, MutexGuard, RwLock};

use account::Account;
use accounts::Accounts;
use block::Block;
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use blockchain_base::{AbstractBlockchain, BlockchainError, Direction};
use database::{Environment, ReadTransaction, Transaction, WriteTransaction};
use genesis::NetworkInfo;
use hash::Blake2bHash;
use keys::Address;
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use primitives::policy;
use primitives::slot::Slots;
use transaction::{Transaction as BlockchainTransaction, TransactionReceipt, TransactionsProof};
use tree_primitives::accounts_proof::AccountsProof;
use tree_primitives::accounts_tree_chunk::AccountsTreeChunk;
use utils::observer::{Listener, ListenerHandle, Notifier};
use utils::time::OffsetTime;

use crate::blockchain_state::BlockchainState;
use crate::chain_info::ChainInfo;
use crate::chain_store::ChainStore;
use crate::reward::genesis_parameters;
use crate::transaction_cache::TransactionCache;
use crate::{BlockchainEvent, ForkEvent, PushError, PushResult};
use std::cmp;
use std::collections::HashSet;

/// The Blockchain struct. It stores all information of the blockchain. It is the main data
/// structure in this crate.
pub struct Blockchain {
    // The environment of the blockchain.
    pub(crate) env: Environment,
    // The network ID. It determines if this is the mainnet or one of the testnets.
    pub network_id: NetworkId,
    // The OffsetTime struct. It allows us query the current time.
    pub time: Arc<OffsetTime>,
    // The notifier processes events relative to the blockchain.
    pub notifier: RwLock<Notifier<'static, BlockchainEvent>>,
    // The fork notifier processes fork events.
    pub fork_notifier: RwLock<Notifier<'static, ForkEvent>>,
    // The chain store is a database containing all of the blocks.
    pub chain_store: Arc<ChainStore>,
    // The current state of the blockchain.
    pub(crate) state: RwLock<BlockchainState>,
    pub(crate) push_lock: Mutex<()>,
    #[cfg(feature = "metrics")]
    pub(crate) metrics: BlockchainMetrics,
    // The coin supply at the genesis block. This is needed to calculate the rewards.
    pub(crate) genesis_supply: Coin,
    // The timestamp at the genesis block. This is needed to calculate the rewards.
    pub(crate) genesis_timestamp: u64,
}

/// Implements methods to start a Blockchain.
impl Blockchain {
    /// Creates a new blockchain from a given environment and network ID.
    pub fn new(env: Environment, network_id: NetworkId) -> Result<Self, BlockchainError> {
        let chain_store = Arc::new(ChainStore::new(env.clone()));

        let time = Arc::new(OffsetTime::new());

        Ok(match chain_store.get_head(None) {
            Some(head_hash) => Blockchain::load(env, network_id, chain_store, time, head_hash)?,
            None => Blockchain::init(env, network_id, chain_store, time)?,
        })
    }

    /// Loads a blockchain from given inputs.
    fn load(
        env: Environment,
        network_id: NetworkId,
        chain_store: Arc<ChainStore>,
        time: Arc<OffsetTime>,
        head_hash: Blake2bHash,
    ) -> Result<Self, BlockchainError> {
        // Check that the correct genesis block is stored.
        let network_info = NetworkInfo::from_network_id(network_id);

        let genesis_info = chain_store.get_chain_info(network_info.genesis_hash(), false, None);

        if !genesis_info
            .as_ref()
            .map(|i| i.on_main_chain)
            .unwrap_or(false)
        {
            return Err(BlockchainError::InvalidGenesisBlock);
        }

        let (genesis_supply, genesis_timestamp) =
            genesis_parameters(&genesis_info.unwrap().head.unwrap_macro().header);

        // Load main chain from store.
        let main_chain = chain_store
            .get_chain_info(&head_hash, true, None)
            .ok_or(BlockchainError::FailedLoadingMainChain)?;

        // Check that chain/accounts state is consistent.
        let accounts = Accounts::new(env.clone());

        if main_chain.head.state_root() != &accounts.hash(None) {
            return Err(BlockchainError::InconsistentState);
        }

        // Load macro chain from store.
        let macro_chain_info = chain_store
            .get_chain_info_at(
                policy::last_macro_block(main_chain.head.block_number()),
                true,
                None,
            )
            .ok_or(BlockchainError::FailedLoadingMainChain)?;

        let macro_head = match macro_chain_info.head {
            Block::Macro(ref macro_head) => macro_head,
            Block::Micro(_) => return Err(BlockchainError::InconsistentState),
        };

        let macro_head_hash = macro_head.hash();

        // Load election macro chain from store.
        let election_chain_info = chain_store
            .get_chain_info_at(
                policy::last_election_block(main_chain.head.block_number()),
                true,
                None,
            )
            .ok_or(BlockchainError::FailedLoadingMainChain)?;

        let election_head = match election_chain_info.head {
            Block::Macro(macro_head) => macro_head,
            Block::Micro(_) => return Err(BlockchainError::InconsistentState),
        };

        if !election_head.is_election_block() {
            return Err(BlockchainError::InconsistentState);
        }

        let election_head_hash = election_head.hash();

        // Initialize TransactionCache.
        let mut transaction_cache = TransactionCache::new();

        let blocks = chain_store.get_blocks_backward(
            &head_hash,
            transaction_cache.missing_blocks() - 1,
            true,
            None,
        );

        for block in blocks.iter().rev() {
            transaction_cache.push_block(block);
        }

        transaction_cache.push_block(&main_chain.head);

        assert_eq!(
            transaction_cache.missing_blocks(),
            policy::TRANSACTION_VALIDITY_WINDOW_ALBATROSS
                .saturating_sub(main_chain.head.block_number() + 1)
        );

        // Current slots and validators
        let current_slots = election_head.get_slots().unwrap();

        // Get last slots and validators
        let prev_block =
            chain_store.get_block(&election_head.header.parent_election_hash, true, None);

        let last_slots = match prev_block {
            Some(Block::Macro(prev_election_block)) => {
                if prev_election_block.is_election_block() {
                    prev_election_block.get_slots().unwrap()
                } else {
                    return Err(BlockchainError::InconsistentState);
                }
            }
            None => Slots::default(),
            _ => return Err(BlockchainError::InconsistentState),
        };

        Ok(Blockchain {
            env,
            network_id,
            time,
            notifier: RwLock::new(Notifier::new()),
            fork_notifier: RwLock::new(Notifier::new()),
            chain_store,
            state: RwLock::new(BlockchainState {
                accounts,
                transaction_cache,
                main_chain,
                head_hash,
                macro_info: macro_chain_info,
                macro_head_hash,
                election_head,
                election_head_hash,
                current_slots: Some(current_slots),
                previous_slots: Some(last_slots),
            }),
            push_lock: Mutex::new(()),

            #[cfg(feature = "metrics")]
            metrics: BlockchainMetrics::default(),
            genesis_supply,
            genesis_timestamp,
        })
    }

    /// Initializes a blockchain.
    fn init(
        env: Environment,
        network_id: NetworkId,
        chain_store: Arc<ChainStore>,
        time: Arc<OffsetTime>,
    ) -> Result<Self, BlockchainError> {
        // Initialize chain & accounts with genesis block.
        let network_info = NetworkInfo::from_network_id(network_id);

        let genesis_block = network_info.genesis_block::<Block>();

        let genesis_macro_block = (match genesis_block {
            Block::Macro(ref macro_block) => Some(macro_block),
            _ => None,
        })
        .unwrap();

        let (genesis_supply, genesis_timestamp) = genesis_parameters(&genesis_macro_block.header);

        let main_chain = ChainInfo::initial(genesis_block.clone());

        let head_hash = network_info.genesis_hash().clone();

        // Initialize accounts.
        let accounts = Accounts::new(env.clone());

        let mut txn = WriteTransaction::new(&env);

        accounts.init(&mut txn, network_info.genesis_accounts());

        // Store genesis block.
        chain_store.put_chain_info(&mut txn, &head_hash, &main_chain, true);

        chain_store.set_head(&mut txn, &head_hash);

        txn.commit();

        // Initialize empty TransactionCache.
        let transaction_cache = TransactionCache::new();

        // current slots and validators
        let current_slots = &genesis_macro_block.get_slots().unwrap();

        let last_slots = Slots::default();

        Ok(Blockchain {
            env,
            network_id,
            time,
            notifier: RwLock::new(Notifier::new()),
            fork_notifier: RwLock::new(Notifier::new()),
            chain_store,
            state: RwLock::new(BlockchainState {
                accounts,
                transaction_cache,
                macro_info: main_chain.clone(),
                main_chain,
                head_hash: head_hash.clone(),
                macro_head_hash: head_hash.clone(),
                election_head: genesis_macro_block.clone(),
                election_head_hash: head_hash,
                current_slots: Some(current_slots.clone()),
                previous_slots: Some(last_slots),
            }),
            push_lock: Mutex::new(()),

            #[cfg(feature = "metrics")]
            metrics: BlockchainMetrics::default(),
            genesis_supply,
            genesis_timestamp,
        })
    }
}

// TODO: To remove this you need to handle Mempool crate (that requires this trait). That probably
// requires separating the 1.0 stuff from the rest of the code.
impl AbstractBlockchain for Blockchain {
    type Block = Block;

    fn new(
        env: Environment,
        network_id: NetworkId,
        _time: Arc<OffsetTime>,
    ) -> Result<Self, BlockchainError> {
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
        self.chain_store.get_block(hash, include_body, None)
    }

    fn get_block_at(&self, height: u32, include_body: bool) -> Option<Self::Block> {
        self.chain_store.get_block_at(height, include_body, None)
    }

    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash> {
        let mut locators: Vec<Blake2bHash> = Vec::with_capacity(max_count);
        let mut hash = self.head_hash();

        // Push top ten hashes.
        locators.push(hash.clone());
        for _ in 0..cmp::min(10, self.block_number()) {
            let block = self.chain_store.get_block(&hash, false, None);
            match block {
                Some(block) => {
                    hash = block.header().parent_hash().clone();
                    locators.push(hash.clone());
                }
                None => break,
            }
        }

        let mut step = 2;
        let mut height = self.block_number().saturating_sub(10 + step);
        let mut opt_block = self.chain_store.get_block_at(height, false, None);
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

    fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
    ) -> Vec<Self::Block> {
        self.chain_store
            .get_blocks(start_block_hash, count, include_body, direction, None)
    }

    fn push(&self, block: Self::Block) -> Result<PushResult, PushError> {
        self.push(block)
    }

    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool {
        self.contains(hash, include_forks)
    }

    #[allow(unused_variables)]
    fn get_accounts_proof(
        &self,
        block_hash: &Blake2bHash,
        addresses: &[Address],
    ) -> Option<AccountsProof<Account>> {
        unimplemented!()
    }

    #[allow(unused_variables)]
    fn get_transactions_proof(
        &self,
        block_hash: &Blake2bHash,
        addresses: &HashSet<Address>,
    ) -> Option<TransactionsProof> {
        unimplemented!()
    }

    #[allow(unused_variables)]
    fn get_transaction_receipts_by_address(
        &self,
        address: &Address,
        sender_limit: usize,
        recipient_limit: usize,
    ) -> Vec<TransactionReceipt> {
        unimplemented!()
    }

    fn register_listener<T: Listener<BlockchainEvent> + 'static>(
        &self,
        listener: T,
    ) -> ListenerHandle {
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

    fn get_accounts_chunk(
        &self,
        prefix: &str,
        size: usize,
        txn_option: Option<&Transaction>,
    ) -> Option<AccountsTreeChunk<Account>> {
        self.state
            .read()
            .accounts
            .get_chunk(prefix, size, txn_option)
    }

    fn get_batch_transactions(
        &self,
        batch: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<BlockchainTransaction>> {
        Blockchain::get_batch_transactions(self, batch, txn_option)
    }

    fn get_epoch_transactions(
        &self,
        epoch: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<BlockchainTransaction>> {
        Blockchain::get_epoch_transactions(self, epoch, txn_option)
    }

    fn validator_registry_address(&self) -> Option<&Address> {
        NetworkInfo::from_network_id(self.network_id).validator_registry_address()
    }
}
