extern crate nimiq_account as account;
extern crate nimiq_block_albatross as block_albatross;
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
use block_albatross::{Block, BlockError};
use database::Environment;
use database::{ReadTransaction, Transaction};
use hash::Blake2bHash;
use keys::Address;
use primitives::networks::NetworkId;
use transaction::Transaction as BlockchainTransaction;
use transaction::{TransactionReceipt, TransactionsProof};
use tree_primitives::accounts_proof::AccountsProof;
use tree_primitives::accounts_tree_chunk::AccountsTreeChunk;
use utils::observer::{Listener, ListenerHandle};
use utils::time::OffsetTime;

#[cfg(feature = "metrics")]
pub mod chain_metrics;

pub trait AbstractBlockchain: Sized + Send + Sync {
    // TODO: Should this be `Block + 'static`? Our implementations would satisfy this anyway. And
    // I think all uses of AbstractBlockchain require it to be 'static too.
    //type VerifyResult;

    // XXX This signature is most likely too restrictive to accommodate all blockchain types.
    fn new(
        env: Environment,
        network_id: NetworkId,
        time: Arc<OffsetTime>,
    ) -> Result<Self, BlockchainError>;

    #[cfg(feature = "metrics")]
    fn metrics(&self) -> &chain_metrics::BlockchainMetrics;

    /// Returns the network ID
    fn network_id(&self) -> NetworkId;

    /// Returns the current head block
    fn head_block(&self) -> MappedRwLockReadGuard<Block>;

    /// Returns the current head hash of the active chain
    fn head_hash(&self) -> Blake2bHash;

    /// Returns the height of the current head
    fn head_height(&self) -> u32;

    /// Get block by hash
    fn get_block(&self, hash: &Blake2bHash, include_body: bool) -> Option<Block>;

    /// Get block by block number
    fn get_block_at(&self, height: u32, include_body: bool) -> Option<Block>;

    /// Get block locators
    fn get_block_locators(&self, max_count: usize) -> Vec<Blake2bHash>;

    /// Get `count` blocks starting from `start_block_hash` into `direction` and optionally include
    /// block bodies.
    fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
    ) -> Vec<Block>;

    /// Verify a block
    //fn verify(&self, block: &Self::Block) -> Self::VerifyResult;

    /// Push a block to the block chain
    fn push(&self, block: Block) -> Result<PushResult, PushError<BlockError>>;

    /// Check if a block with the given hash is included in the blockchain
    /// `include_forks` will also check for micro-block forks in the current epoch
    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool;

    fn get_accounts_proof(
        &self,
        block_hash: &Blake2bHash,
        addresses: &[Address],
    ) -> Option<AccountsProof<Account>>;

    fn get_transactions_proof(
        &self,
        block_hash: &Blake2bHash,
        addresses: &HashSet<Address>,
    ) -> Option<TransactionsProof>;

    fn get_transaction_receipts_by_address(
        &self,
        address: &Address,
        sender_limit: usize,
        recipient_limit: usize,
    ) -> Vec<TransactionReceipt>;

    /* Required by Mempool */

    fn register_listener<T: Listener<BlockchainEvent<Block>> + 'static>(
        &self,
        listener: T,
    ) -> ListenerHandle;

    fn lock(&self) -> MutexGuard<()>;

    fn get_account(&self, address: &Address) -> Account;

    fn contains_tx_in_validity_window(&self, tx_hash: &Blake2bHash) -> bool;

    /* Required by AccountsChunkCache */
    // TODO Why do we need this? Remove if possible.
    fn head_hash_from_store(&self, txn: &ReadTransaction) -> Option<Blake2bHash>;

    fn get_accounts_chunk(
        &self,
        prefix: &str,
        size: usize,
        txn_option: Option<&Transaction>,
    ) -> Option<AccountsTreeChunk<Account>>;

    // TODO: Currently, we can implement request responses in the ConsensusAgent only for *both* protocols, which is why AbstractBlockchain needs to support this.
    fn get_batch_transactions(
        &self,
        batch: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<BlockchainTransaction>>;

    // TODO: Currently, we can implement request responses in the ConsensusAgent only for *both* protocols, which is why AbstractBlockchain needs to support this.
    fn get_epoch_transactions(
        &self,
        epoch: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<BlockchainTransaction>>;

    fn validator_registry_address(&self) -> Option<&Address>;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockchainEvent<BL> {
    Extended(Blake2bHash),
    Rebranched(Vec<(Blake2bHash, BL)>, Vec<(Blake2bHash, BL)>),
    Finalized(Blake2bHash),
    EpochFinalized(Blake2bHash),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError<BE> {
    Orphan,
    InvalidBlock(BE),
    InvalidSuccessor,

    DuplicateTransaction,

    AccountsError(AccountError),

    InvalidFork,

    BlockchainError(BlockchainError),
}

impl<BE> PushError<BE> {
    /// Create a `PushError` from a `BlockError`.
    ///
    /// NOTE: We can't implement `From<BE: BlockError>`, since the compiler can't guarantee that
    /// nobody will implement `BlockError` for `AccountError`, which would result in a duplicate
    /// implementation.
    pub fn from_block_error(e: BE) -> Self {
        PushError::InvalidBlock(e)
    }
}

impl<BE> From<AccountError> for PushError<BE> {
    fn from(e: AccountError) -> Self {
        PushError::AccountsError(e)
    }
}

impl<BE> From<BlockchainError> for PushError<BE> {
    fn from(e: BlockchainError) -> Self {
        PushError::BlockchainError(e)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Direction {
    Forward,
    Backward,
}
