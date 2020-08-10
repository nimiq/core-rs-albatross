use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use accounts::Accounts;
use block::Block;
#[cfg(feature = "metrics")]
use blockchain_base::chain_metrics::BlockchainMetrics;
use blockchain_base::BlockchainError;
use database::{Environment, WriteTransaction};
use genesis::NetworkInfo;
use hash::Blake2bHash;
use primitives::coin::Coin;
use primitives::networks::NetworkId;
use primitives::policy;
use primitives::slot::Slots;
use utils::observer::Notifier;
use utils::time::OffsetTime;

use crate::blockchain_state::BlockchainState;
use crate::chain_info::ChainInfo;
use crate::chain_store::ChainStore;
use crate::reward::genesis_parameters;
use crate::transaction_cache::TransactionCache;
use crate::{BlockchainEvent, ForkEvent};

pub struct Blockchain {
    pub(crate) env: Environment,
    pub network_id: NetworkId,
    pub time: Arc<OffsetTime>,
    pub notifier: RwLock<Notifier<'static, BlockchainEvent>>,
    pub fork_notifier: RwLock<Notifier<'static, ForkEvent>>,
    pub chain_store: Arc<ChainStore>,
    pub(crate) state: RwLock<BlockchainState>,
    pub(crate) push_lock: Mutex<()>,

    #[cfg(feature = "metrics")]
    pub(crate) metrics: BlockchainMetrics,

    pub(crate) genesis_supply: Coin,
    pub(crate) genesis_timestamp: u64,
}

// complicated stuff
impl Blockchain {
    pub fn new(env: Environment, network_id: NetworkId) -> Result<Self, BlockchainError> {
        let chain_store = Arc::new(ChainStore::new(env.clone()));
        let time = Arc::new(OffsetTime::new());
        Ok(match chain_store.get_head(None) {
            Some(head_hash) => Blockchain::load(env, network_id, chain_store, time, head_hash)?,
            None => Blockchain::init(env, network_id, chain_store, time)?,
        })
    }

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
            genesis_parameters(genesis_info.unwrap().head.unwrap_macro_ref());

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
        let current_slots = election_head.get_slots();

        // Get last slots and validators
        let prev_block =
            chain_store.get_block(&election_head.header.parent_election_hash, true, None);
        let last_slots = match prev_block {
            Some(Block::Macro(prev_election_block)) => {
                if prev_election_block.is_election_block() {
                    prev_election_block.get_slots()
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
        let (genesis_supply, genesis_timestamp) = genesis_parameters(genesis_macro_block);
        let main_chain = ChainInfo::initial(genesis_block.clone());
        let head_hash = network_info.genesis_hash().clone();

        // Initialize accounts.
        let accounts = Accounts::new(env.clone());
        let mut txn = WriteTransaction::new(&env);
        accounts.init(&mut txn, network_info.genesis_accounts());

        // Commit genesis block to accounts.
        // XXX Don't distribute any reward for the genesis block, so there is nothing to commit.

        // Store genesis block.
        chain_store.put_chain_info(&mut txn, &head_hash, &main_chain, true);
        chain_store.set_head(&mut txn, &head_hash);
        txn.commit();

        // Initialize empty TransactionCache.
        let transaction_cache = TransactionCache::new();

        // current slots and validators
        let current_slots = &genesis_macro_block.get_slots();
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
