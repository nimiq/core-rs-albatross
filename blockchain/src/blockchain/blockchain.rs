use std::sync::Arc;

use nimiq_account::{Accounts, BlockLog};
use nimiq_block::Block;
use nimiq_blockchain_interface::{BlockchainError, BlockchainEvent, ChainInfo, ForkEvent};
use nimiq_database::{
    traits::{Database, WriteTransaction},
    DatabaseProxy, TransactionProxy, WriteTransactionProxy,
};
use nimiq_genesis::NetworkInfo;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{
    coin::Coin, networks::NetworkId, policy::Policy, slots_allocation::Validators, trie::TrieItem,
};
use nimiq_utils::time::OffsetTime;
use tokio::sync::broadcast::{channel as broadcast, Sender as BroadcastSender};

#[cfg(feature = "metrics")]
use crate::chain_metrics::BlockchainMetrics;
use crate::{
    blockchain_state::BlockchainState, chain_store::ChainStore, history::HistoryStore,
    history_store_proxy::HistoryStoreProxy, interface::HistoryInterface,
    reward::genesis_parameters, HistoryStoreIndex,
};

const BROADCAST_MAX_CAPACITY: usize = 256;

/// The Blockchain struct. It stores all information of the blockchain. It is the main data
/// structure in this crate.
pub struct Blockchain {
    /// The environment of the blockchain.
    pub(crate) env: DatabaseProxy,
    /// Blockchain configuration options
    pub config: BlockchainConfig,
    /// The network ID. It determines if this is the mainnet or one of the testnets.
    pub network_id: NetworkId,
    /// The OffsetTime struct. It allows us to query the current time.
    pub time: Arc<OffsetTime>, // shared with network
    /// The notifier processes events relative to the blockchain.
    pub notifier: BroadcastSender<BlockchainEvent>,
    /// The fork notifier processes fork events.
    pub fork_notifier: BroadcastSender<ForkEvent>,
    /// The log notifier processes all events regarding accounts changes.
    pub log_notifier: BroadcastSender<BlockLog>,
    /// The chain store is a database containing all of the chain infos, blocks and receipts.
    pub chain_store: ChainStore,
    /// The history store is a database containing all of the history trees and transactions.
    pub history_store: HistoryStoreProxy,
    /// The current state of the blockchain.
    pub state: BlockchainState,
    /// A reference to a "function" to test whether a given transaction is known and valid.
    pub tx_verification_cache: Arc<dyn TransactionVerificationCache>,
    /// The metrics for the blockchain. Needed for analysis.
    #[cfg(feature = "metrics")]
    pub(crate) metrics: Arc<BlockchainMetrics>,
    /// The coin supply at the genesis block. This is needed to calculate the rewards.
    pub(crate) genesis_supply: Coin,
    /// The timestamp at the genesis block. This is needed to calculate the rewards.
    pub(crate) genesis_timestamp: u64,
    /// The Genesis block number
    pub(crate) genesis_block_number: u32,
    /// The Genesis hash used for various checks
    pub(crate) genesis_hash: Blake2bHash,
}

/// Contains various blockchain configuration knobs
pub struct BlockchainConfig {
    /// Flag indicating if the full history should be stored
    pub keep_history: bool,
    /// Maximum number of epochs (other than the current one) that the ChainStore will store fully.
    /// Epochs older than this number will be pruned.
    pub max_epochs_stored: u32,
    /// Enables/Disables indices in the history store.
    pub index_history: bool,
}

impl Default for BlockchainConfig {
    fn default() -> Self {
        Self {
            keep_history: true,
            max_epochs_stored: Policy::MIN_EPOCHS_STORED,
            index_history: true,
        }
    }
}

/// Implements methods to start a Blockchain.
impl Blockchain {
    /// Creates a new blockchain from a given environment and network ID.
    pub fn new(
        env: DatabaseProxy,
        config: BlockchainConfig,
        network_id: NetworkId,
        time: Arc<OffsetTime>,
    ) -> Result<Self, BlockchainError> {
        let network_info = NetworkInfo::from_network_id(network_id);
        let genesis_block = network_info.genesis_block();
        let genesis_accounts = network_info.genesis_accounts();

        Self::with_genesis(
            env,
            config,
            time,
            network_id,
            genesis_block,
            genesis_accounts,
        )
    }

    /// Creates a new blockchain with the given genesis block.
    pub fn with_genesis(
        env: DatabaseProxy,
        config: BlockchainConfig,
        time: Arc<OffsetTime>,
        network_id: NetworkId,
        genesis_block: Block,
        genesis_accounts: Vec<TrieItem>,
    ) -> Result<Self, BlockchainError> {
        if !Policy::is_election_block_at(genesis_block.block_number()) {
            log::error!(
                genesis_block_number = genesis_block.block_number(),
                "The genesis block number must correspond to an election block"
            );
            return Err(BlockchainError::InvalidGenesisBlock);
        }

        if Policy::genesis_block_number() != genesis_block.block_number() {
            log::error!("The genesis block number and the one configured in Policy must be equal");
            return Err(BlockchainError::InvalidGenesisBlock);
        }

        if network_id != genesis_block.network() {
            log::error!(
                "The genesis block network ID must match the one configured for the blockchain"
            );
            return Err(BlockchainError::InvalidGenesisBlock);
        }

        let chain_store = ChainStore::new(env.clone());

        Ok(match chain_store.get_head(None) {
            Some(head_hash) => Blockchain::load(
                env,
                config,
                chain_store,
                time,
                network_id,
                genesis_block,
                head_hash,
            )?,
            None => Blockchain::init(
                env,
                config,
                chain_store,
                time,
                network_id,
                genesis_block,
                genesis_accounts,
            )?,
        })
    }

    /// Loads a blockchain from given inputs.
    fn load(
        env: DatabaseProxy,
        config: BlockchainConfig,
        chain_store: ChainStore,

        time: Arc<OffsetTime>,
        network_id: NetworkId,
        genesis_block: Block,
        head_hash: Blake2bHash,
    ) -> Result<Self, BlockchainError> {
        // Check that the correct genesis block is stored.
        let genesis_info = chain_store.get_chain_info(&genesis_block.hash(), false, None);
        if !genesis_info
            .as_ref()
            .map(|i| i.on_main_chain)
            .unwrap_or(false)
        {
            return Err(BlockchainError::InvalidGenesisBlock);
        }

        let genesis_block_number = genesis_block.block_number();
        let genesis_hash = genesis_block.hash();

        let (genesis_supply, genesis_timestamp) =
            genesis_parameters(&genesis_block.unwrap_macro().header);

        // Load main chain from store.
        let main_chain = chain_store
            .get_chain_info(&head_hash, true, None)
            .map_err(|_| BlockchainError::FailedLoadingMainChain)?;

        // Check that chain/accounts state is consistent.
        let accounts = Accounts::new(env.clone());

        // Verify accounts hash if the tree is complete or changes only happened in the complete part.
        if let Some(accounts_hash) = accounts.get_root_hash(None) {
            if main_chain.head.state_root() != &accounts_hash {
                log::error!(
                    "Main chain's head state root: {:?}, Account state root: {:?}",
                    main_chain.head.state_root(),
                    &accounts_hash
                );
                return Err(BlockchainError::InconsistentState);
            }
        }

        // Load macro chain from store.
        let macro_chain_info = chain_store
            .get_chain_info_at(
                Policy::last_macro_block(main_chain.head.block_number()),
                true,
                None,
            )
            .map_err(|_| BlockchainError::FailedLoadingMainChain)?;

        let macro_head = match macro_chain_info.head {
            Block::Macro(ref macro_head) => macro_head,
            Block::Micro(_) => return Err(BlockchainError::InconsistentState),
        };

        let macro_head_hash = macro_head.hash();

        // Load election macro chain from store.
        let election_chain_info = chain_store
            .get_chain_info_at(
                Policy::last_election_block(main_chain.head.block_number()),
                true,
                None,
            )
            .map_err(|_| BlockchainError::FailedLoadingMainChain)?;

        let election_head = match election_chain_info.head {
            Block::Macro(macro_head) => macro_head,
            Block::Micro(_) => return Err(BlockchainError::InconsistentState),
        };

        if !election_head.is_election() {
            return Err(BlockchainError::InconsistentState);
        }

        let election_head_hash = election_head.hash();

        // Current slots and validators
        let current_slots = election_head.get_validators().unwrap();

        // Get last slots and validators
        let prev_block =
            chain_store.get_block(&election_head.header.parent_election_hash, true, None);

        let last_slots = match prev_block {
            Ok(Block::Macro(prev_election_block)) => prev_election_block.get_validators(),
            Ok(_) => {
                // Expected a macro block and received a micro block
                return Err(BlockchainError::InconsistentState);
            }
            Err(_) => {
                // Block wasn't found
                None
            }
        };

        let (tx, _rx) = broadcast(BROADCAST_MAX_CAPACITY);
        let (tx_fork, _rx_fork) = broadcast(BROADCAST_MAX_CAPACITY);
        let (tx_log, _rx_log) = broadcast(BROADCAST_MAX_CAPACITY);

        let history_store = if config.index_history {
            HistoryStoreProxy::WithIndex(HistoryStoreIndex::new(env.clone(), network_id))
        } else {
            HistoryStoreProxy::WithoutIndex(Box::new(HistoryStore::new(env.clone(), network_id))
                as Box<dyn HistoryInterface + Sync + Send>)
        };

        Ok(Blockchain {
            env,
            config,
            network_id,
            time,
            notifier: tx,
            fork_notifier: tx_fork,
            log_notifier: tx_log,
            chain_store,
            history_store,
            state: BlockchainState {
                accounts,
                main_chain,
                head_hash,
                macro_info: macro_chain_info,
                macro_head_hash,
                election_head,
                election_head_hash,
                current_slots: Some(current_slots),
                previous_slots: last_slots,
            },
            tx_verification_cache: Arc::new(DEFAULT_TX_VERIFICATION_CACHE),
            #[cfg(feature = "metrics")]
            metrics: Arc::new(BlockchainMetrics::default()),
            genesis_supply,
            genesis_timestamp,
            genesis_block_number,
            genesis_hash,
        })
    }

    /// Initializes a blockchain.
    fn init(
        env: DatabaseProxy,
        config: BlockchainConfig,
        chain_store: ChainStore,

        time: Arc<OffsetTime>,
        network_id: NetworkId,
        genesis_block: Block,
        genesis_accounts: Vec<TrieItem>,
    ) -> Result<Self, BlockchainError> {
        // Initialize chain & accounts with genesis block.
        let head_hash = genesis_block.hash();

        let genesis_macro_block = genesis_block.unwrap_macro_ref().clone();
        let genesis_block_number = genesis_block.block_number();
        let genesis_hash = genesis_block.hash();
        let current_slots = genesis_macro_block.get_validators().expect("Slots missing");
        let (genesis_supply, genesis_timestamp) = genesis_parameters(&genesis_macro_block.header);

        let main_chain = ChainInfo::new(genesis_block, true);

        // Initialize accounts.
        let accounts = Accounts::new(env.clone());
        let mut txn = env.write_transaction();
        accounts.init(&mut (&mut txn).into(), genesis_accounts);

        // Store genesis block.
        chain_store.put_chain_info(&mut txn, &head_hash, &main_chain, true);
        chain_store.set_head(&mut txn, &head_hash);
        txn.commit();

        let (tx, _rx) = broadcast(BROADCAST_MAX_CAPACITY);
        let (tx_fork, _rx_fork) = broadcast(BROADCAST_MAX_CAPACITY);
        let (tx_log, _rx_log) = broadcast(BROADCAST_MAX_CAPACITY);

        let history_store = if config.index_history {
            HistoryStoreProxy::WithIndex(HistoryStoreIndex::new(env.clone(), network_id))
        } else {
            HistoryStoreProxy::WithoutIndex(Box::new(HistoryStore::new(env.clone(), network_id))
                as Box<dyn HistoryInterface + Sync + Send>)
        };

        Ok(Blockchain {
            env,
            config,
            network_id,
            time,
            notifier: tx,
            fork_notifier: tx_fork,
            log_notifier: tx_log,
            chain_store,
            history_store,
            state: BlockchainState {
                accounts,
                macro_info: main_chain.clone(),
                main_chain,
                head_hash: head_hash.clone(),
                macro_head_hash: head_hash.clone(),
                election_head: genesis_macro_block,
                election_head_hash: head_hash,
                current_slots: Some(current_slots),
                previous_slots: Some(Validators::default()),
            },
            tx_verification_cache: Arc::new(DEFAULT_TX_VERIFICATION_CACHE),
            #[cfg(feature = "metrics")]
            metrics: Arc::new(BlockchainMetrics::default()),
            genesis_supply,
            genesis_timestamp,
            genesis_block_number,
            genesis_hash,
        })
    }

    pub fn get_genesis_parameters(&self) -> (Coin, u64) {
        (self.genesis_supply, self.genesis_timestamp)
    }

    pub fn get_genesis_block_number(&self) -> u32 {
        self.genesis_block_number
    }

    pub fn read_transaction(&self) -> TransactionProxy {
        self.env.read_transaction()
    }

    pub fn write_transaction(&self) -> WriteTransactionProxy {
        self.env.write_transaction()
    }
}

pub trait TransactionVerificationCache: Send + Sync {
    fn is_known(&self, tx_hash: &Blake2bHash) -> bool;
}

struct DefaultTransactionVerificationCache {}

impl TransactionVerificationCache for DefaultTransactionVerificationCache {
    fn is_known(&self, _: &Blake2bHash) -> bool {
        false
    }
}

const DEFAULT_TX_VERIFICATION_CACHE: DefaultTransactionVerificationCache =
    DefaultTransactionVerificationCache {};
