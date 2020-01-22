extern crate nimiq_account as account;
extern crate nimiq_block_base as block_base;
extern crate nimiq_database as database;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_tree_primitives as tree_primitives;
extern crate nimiq_utils as utils;

use std::collections::HashSet;
use std::fmt::Debug;
use std::sync::Arc;

use failure::Fail;
use parking_lot::MappedRwLockReadGuard;
use parking_lot::MutexGuard;

use account::{Account, AccountError};
use block_base::{Block, BlockError};
use database::{ReadTransaction, Transaction};
use database::Environment;
use hash::Blake2bHash;
use keys::Address;
use nimiq_network_primitives::time::NetworkTime;
use primitives::networks::NetworkId;
use transaction::{TransactionReceipt, TransactionsProof};
use transaction::Transaction as BlockchainTransaction;
use tree_primitives::accounts_proof::AccountsProof;
use tree_primitives::accounts_tree_chunk::AccountsTreeChunk;
use utils::observer::{Listener, ListenerHandle};

#[cfg(feature = "metrics")]
pub mod chain_metrics;

pub trait AbstractBlockchain: Sized + Send + Sync {
    // TODO: Should this be `Block + 'static`? Our implementations would satisfy this anyway. And
    // I think all uses of AbstractBlockchain require it to be 'static too.
    type Block: Block;
    //type VerifyResult;

    // XXX This signature is most likely too restrictive to accommodate all blockchain types.
    fn new(env: Environment, network_id: NetworkId, network_time: Arc<NetworkTime>) -> Result<Self, BlockchainError>;


    #[cfg(feature = "metrics")]
    fn metrics(&self) -> &chain_metrics::BlockchainMetrics;


    /// Returns the network ID
    fn network_id(&self) -> NetworkId;


    /// Returns the current head block
    fn head_block(&self) -> MappedRwLockReadGuard<Self::Block>;

    /// Returns the current head hash of the active chain
    fn head_hash(&self) -> Blake2bHash;

    /// Returns the height of the current head
    fn head_height(&self) -> u32;


    /// Get block by hash
    fn get_block(&self, hash: &Blake2bHash, include_body: bool) -> Option<Self::Block>;

    /// Get block by block number
    fn get_block_at(&self, height: u32, include_body: bool) -> Option<Self::Block>;

    /// Get block locators
    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash>;

    /// Get `count` blocks starting from `start_block_hash` into `direction` and optionally include
    /// block bodies.
    fn get_blocks(&self, start_block_hash: &Blake2bHash, count: u32, include_body: bool, direction: Direction) -> Vec<Self::Block>;


    /// Verify a block
    //fn verify(&self, block: &Self::Block) -> Self::VerifyResult;

    /// Push a block to the block chain
    fn push(&self, block: Self::Block) -> Result<PushResult, PushError<<Self::Block as Block>::Error>>;


    /// Check if a block with the given hash is included in the blockchain
    /// `include_forks` will also check for micro-block forks in the current epoch
    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool;


    fn get_accounts_proof(&self, block_hash: &Blake2bHash, addresses: &[Address]) -> Option<AccountsProof<Account>>;

    fn get_transactions_proof(&self, block_hash: &Blake2bHash, addresses: &HashSet<Address>) -> Option<TransactionsProof>;

    fn get_transaction_receipts_by_address(&self, address: &Address, sender_limit: usize, recipient_limit: usize) -> Vec<TransactionReceipt>;


    /* Required by Mempool */

    fn register_listener<T: Listener<BlockchainEvent<Self::Block>> + 'static>(&self, listener: T) -> ListenerHandle;

    fn lock(&self) -> MutexGuard<()>;

    fn get_account(&self, address: &Address) -> Account;

    fn contains_tx_in_validity_window(&self, tx_hash: &Blake2bHash) -> bool;


    /* Required by AccountsChunkCache */
    // TODO Why do we need this? Remove if possible.
    fn head_hash_from_store(&self, txn: &ReadTransaction) -> Option<Blake2bHash>;

    fn get_accounts_chunk(&self, prefix: &str, size: usize, txn_option: Option<&Transaction>) -> Option<AccountsTreeChunk<Account>>;

    // TODO: Currently, we can implement request responses in the ConsensusAgent only for *both* protocols, which is why AbstractBlockchain needs to support this.
    fn get_epoch_transactions(&self, epoch: u32, txn_option: Option<&Transaction>) -> Option<Vec<BlockchainTransaction>>;

    fn validator_registry_address(&self) -> Option<&Address>;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockchainEvent<BL: Block> {
    Extended(Blake2bHash),
    Rebranched(Vec<(Blake2bHash, BL)>, Vec<(Blake2bHash, BL)>),
    Finalized(Blake2bHash),
}

#[derive(Debug, Fail, Clone, PartialEq, Eq)]
pub enum BlockchainError {
    #[fail(display = "Invalid genesis block stored. Are you on the right network?")]
    InvalidGenesisBlock,
    #[fail(display = "Failed to load the main chain. Reset your consensus database.")]
    FailedLoadingMainChain,
    #[fail(display = "Inconsistent chain/accounts state. Reset your consensus database.")]
    InconsistentState,
    #[fail(display = "No network for: {:?}", _0)]
    NoNetwork(NetworkId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushResult {
    Known,
    Extended,
    Rebranched,
    Forked,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq, Fail)]
pub enum PushError<BE: BlockError> {
    #[fail(display = "Block is an orphan")]
    Orphan,
    #[fail(display = "Block is invalid: {}", _0)]
    InvalidBlock(#[cause] BE),
    #[fail(display = "Block is not a valid successor")]
    InvalidSuccessor,

    // TODO: This is part of powchain's and albatross' BlockError anyway
    #[fail(display = "Block contains duplicate transactions")]
    DuplicateTransaction,

    #[fail(display = "Block can't be applied to accounts tree: {}", _0)]
    AccountsError(#[cause] AccountError),

    #[fail(display = "Block is part of an invalid fork")]
    InvalidFork,

    #[fail(display = "Failed to push block onto block chain: {}", _0)]
    BlockchainError(#[cause] BlockchainError),
}

impl<BE: BlockError> PushError<BE> {
    /// Create a `PushError` from a `BlockError`.
    ///
    /// NOTE: We can't implement `From<BE: BlockError>`, since the compiler can't guarantee that
    /// nobody will implement `BlockError` for `AccountError`, which would result in a duplicate
    /// implementation.
    pub fn from_block_error(e: BE) -> Self {
        PushError::InvalidBlock(e)
    }
}

impl<BE: BlockError> From<AccountError> for PushError<BE> {
    fn from(e: AccountError) -> Self {
        PushError::AccountsError(e)
    }
}

impl<BE: BlockError> From<BlockchainError> for PushError<BE> {
    fn from(e: BlockchainError) -> Self {
        PushError::BlockchainError(e)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Direction {
    Forward,
    Backward,
}
