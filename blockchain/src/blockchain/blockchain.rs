use std::sync::Arc;

use nimiq_account::{Account, Accounts};
use nimiq_block::Block;
use nimiq_database::{Environment, ReadTransaction, WriteTransaction};
use nimiq_genesis::NetworkInfo;
use nimiq_hash::Blake3Hash;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::policy;
use nimiq_primitives::slots::Validators;
use nimiq_utils::observer::Notifier;
use nimiq_utils::time::OffsetTime;

use crate::blockchain_state::BlockchainState;
use crate::chain_info::ChainInfo;
#[cfg(feature = "metrics")]
use crate::chain_metrics::BlockchainMetrics;
use crate::chain_store::ChainStore;
use crate::history_store::HistoryStore;
use crate::reward::genesis_parameters;
use crate::{BlockchainError, BlockchainEvent, ForkEvent};
use nimiq_trie::key_nibbles::KeyNibbles;

/// The Blockchain struct. It stores all information of the blockchain. It is the main data
/// structure in this crate.
pub struct Blockchain {
    // The environment of the blockchain.
    env: Environment,
    // The network ID. It determines if this is the mainnet or one of the testnets.
    pub network_id: NetworkId,
    // The OffsetTime struct. It allows us to query the current time.
    pub time: Arc<OffsetTime>, // shared with network
    // The notifier processes events relative to the blockchain.
    pub notifier: Notifier<BlockchainEvent>,
    // The fork notifier processes fork events.
    pub fork_notifier: Notifier<ForkEvent>,
    // The chain store is a database containing all of the chain infos, blocks and receipts.
    pub chain_store: ChainStore,
    // The history store is a database containing all of the history trees and transactions.
    pub history_store: HistoryStore,
    // The current state of the blockchain.
    pub state: BlockchainState,
    // A reference to a "function" to test whether a given transaction is known and valid.
    pub tx_verification_cache: Arc<dyn TransactionVerificationCache>,
    // The metrics for the blockchain. Needed for analysis.
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
    pub fn new(
        env: Environment,
        network_id: NetworkId,
        time: Arc<OffsetTime>,
    ) -> Result<Self, BlockchainError> {
        let network_info = NetworkInfo::from_network_id(network_id);
        let genesis_block = network_info.genesis_block::<Block>();
        let genesis_accounts = network_info.genesis_accounts();
        Self::with_genesis(env, time, network_id, genesis_block, genesis_accounts)
    }

    /// Creates a new blockchain with the given genesis block.
    pub fn with_genesis(
        env: Environment,
        time: Arc<OffsetTime>,
        network_id: NetworkId,
        genesis_block: Block,
        genesis_accounts: Vec<(KeyNibbles, Account)>,
    ) -> Result<Self, BlockchainError> {
        let chain_store = ChainStore::new(env.clone());
        let history_store = HistoryStore::new(env.clone());

        Ok(match chain_store.get_head(None) {
            Some(head_hash) => Blockchain::load(
                env,
                chain_store,
                history_store,
                time,
                network_id,
                genesis_block,
                head_hash,
            )?,
            None => Blockchain::init(
                env,
                chain_store,
                history_store,
                time,
                network_id,
                genesis_block,
                genesis_accounts,
            )?,
        })
    }

    /// Loads a blockchain from given inputs.
    fn load(
        env: Environment,
        chain_store: ChainStore,
        history_store: HistoryStore,
        time: Arc<OffsetTime>,
        network_id: NetworkId,
        genesis_block: Block,
        head_hash: Blake3Hash,
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

        let (genesis_supply, genesis_timestamp) =
            genesis_parameters(&genesis_block.unwrap_macro().header);

        // Load main chain from store.
        let main_chain = chain_store
            .get_chain_info(&head_hash, true, None)
            .ok_or(BlockchainError::FailedLoadingMainChain)?;

        // Check that chain/accounts state is consistent.
        let accounts = Accounts::new(env.clone());

        if main_chain.head.state_root() != &accounts.get_root(None) {
            log::error!(
                "Main chain's head state root: {:?}, Account state root: {:?}",
                main_chain.head.state_root(),
                &accounts.get_root(None)
            );
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

        // Current slots and validators
        let current_slots = election_head.get_validators().unwrap();

        // Get last slots and validators
        let prev_block =
            chain_store.get_block(&election_head.header.parent_election_hash, true, None);

        let last_slots = match prev_block {
            Some(Block::Macro(prev_election_block)) => prev_election_block.get_validators(),
            None => None,
            _ => return Err(BlockchainError::InconsistentState),
        };

        Ok(Blockchain {
            env,
            network_id,
            time,
            notifier: Notifier::new(),
            fork_notifier: Notifier::new(),
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
            metrics: BlockchainMetrics::default(),
            genesis_supply,
            genesis_timestamp,
        })
    }

    /// Initializes a blockchain.
    fn init(
        env: Environment,
        chain_store: ChainStore,
        history_store: HistoryStore,
        time: Arc<OffsetTime>,
        network_id: NetworkId,
        genesis_block: Block,
        genesis_accounts: Vec<(KeyNibbles, Account)>,
    ) -> Result<Self, BlockchainError> {
        // Initialize chain & accounts with genesis block.
        let head_hash = genesis_block.hash();

        let genesis_macro_block = genesis_block.unwrap_macro_ref().clone();
        let current_slots = genesis_macro_block.get_validators().expect("Slots missing");
        let (genesis_supply, genesis_timestamp) = genesis_parameters(&genesis_macro_block.header);

        let main_chain = ChainInfo::new(genesis_block, true);

        // Initialize accounts.
        let accounts = Accounts::new(env.clone());
        let mut txn = WriteTransaction::new(&env);
        accounts.init(&mut txn, genesis_accounts);

        // Store genesis block.
        chain_store.put_chain_info(&mut txn, &head_hash, &main_chain, true);
        chain_store.set_head(&mut txn, &head_hash);
        txn.commit();

        Ok(Blockchain {
            env,
            network_id,
            time,
            notifier: Notifier::new(),
            fork_notifier: Notifier::new(),
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
            metrics: BlockchainMetrics::default(),
            genesis_supply,
            genesis_timestamp,
        })
    }

    pub fn read_transaction(&self) -> ReadTransaction {
        ReadTransaction::new(&self.env)
    }

    pub fn write_transaction(&self) -> WriteTransaction {
        WriteTransaction::new(&self.env)
    }
}

pub trait TransactionVerificationCache: Send + Sync {
    fn is_known(&self, tx_hash: &Blake3Hash) -> bool;
}

struct DefaultTransactionVerificationCache {}

impl TransactionVerificationCache for DefaultTransactionVerificationCache {
    fn is_known(&self, _: &Blake3Hash) -> bool {
        false
    }
}

const DEFAULT_TX_VERIFICATION_CACHE: DefaultTransactionVerificationCache =
    DefaultTransactionVerificationCache {};
