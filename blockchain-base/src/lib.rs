extern crate nimiq_account as account;
extern crate nimiq_block_base as block_base;
extern crate nimiq_database as database;
extern crate nimiq_hash as hash;
extern crate nimiq_keys as keys;
extern crate nimiq_primitives as primitives;
extern crate nimiq_transaction as transaction;
extern crate nimiq_tree_primitives as tree_primitives;
extern crate nimiq_utils as utils;

#[cfg(feature = "metrics")]
pub mod chain_metrics;

use std::collections::HashSet;
use std::fmt::Debug;
use std::sync::Arc;

use failure::Fail;
use parking_lot::MutexGuard;

use account::{Account, AccountError};
use block_base::Block;
use database::{ReadTransaction, Transaction};
use database::Environment;
use hash::Blake2bHash;
use keys::Address;
use nimiq_network_primitives::time::NetworkTime;
use primitives::networks::NetworkId;
use transaction::{TransactionReceipt, TransactionsProof};
use tree_primitives::accounts_proof::AccountsProof;
use tree_primitives::accounts_tree_chunk::AccountsTreeChunk;
use utils::observer::{Listener, ListenerHandle};

pub trait AbstractBlockchain<'env>: Sized + Send + Sync {
    type Block: Block;
    //type VerifyResult;

    // XXX This signature is most likely too restrictive to accommodate all blockchain types.
    fn new(env: &'env Environment, network_id: NetworkId, network_time: Arc<NetworkTime>) -> Result<Self, BlockchainError>;


    #[cfg(feature = "metrics")]
    fn metrics(&self) -> &chain_metrics::BlockchainMetrics;


    /// Returns the network ID
    fn network_id(&self) -> NetworkId;


    /// Returns the current head block
    fn head_block(&self) -> Self::Block;

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


    fn get_accounts_proof(&self, block_hash: &Blake2bHash, addresses: &[Address]) -> Option<AccountsProof>;

    fn get_transactions_proof(&self, block_hash: &Blake2bHash, addresses: &HashSet<Address>) -> Option<TransactionsProof>;

    fn get_transaction_receipts_by_address(&self, address: &Address, sender_limit: usize, recipient_limit: usize) -> Vec<TransactionReceipt>;


    /* Required by Mempool */

    fn register_listener<T: Listener<BlockchainEvent<Self::Block>> + 'env>(&self, listener: T) -> ListenerHandle;

    fn lock(&self) -> MutexGuard<()>;

    fn get_account(&self, address: &Address) -> Account;

    fn contains_tx_in_validity_window(&self, tx_hash: &Blake2bHash) -> bool;


    /* Required by AccountsChunkCache */
    // TODO Why do we need this? Remove if possible.
    fn head_hash_from_store(&self, txn: &ReadTransaction) -> Option<Blake2bHash>;

    fn get_accounts_chunk(&self, prefix: &str, size: usize, txn_option: Option<&Transaction>) -> Option<AccountsTreeChunk>;
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BlockchainEvent<BL: Block> {
    Extended(Blake2bHash),
    Rebranched(Vec<(Blake2bHash, BL)>, Vec<(Blake2bHash, BL)>),
    Finalized,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError<BE: Debug + Clone + PartialEq + Eq> {
    Orphan,
    InvalidBlock(BE),
    InvalidSuccessor,
    DifficultyMismatch,
    DuplicateTransaction,
    AccountsError(AccountError),
    InvalidFork,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Direction {
    Forward,
    Backward,
}
