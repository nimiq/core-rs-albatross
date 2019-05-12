use std::cmp;
use std::sync::Arc;

use parking_lot::{MappedRwLockReadGuard, Mutex, RwLock, RwLockReadGuard};

use accounts::Accounts;
use bls::bls12_381::{Signature, PublicKey};
use block::{Block, BlockType, BlockError, MacroBlock, ValidatorSlots, MicroBlock};
use block::ViewChange;
use database::{Environment, ReadTransaction, WriteTransaction};
use hash::{Blake2bHash, Hash};
use network_primitives::networks::NetworkInfo;
use network_primitives::time::NetworkTime;
use account::AccountError;
use primitives::networks::NetworkId;
use primitives::slot::Slot;
use primitives::policy;

use utils::observer::Notifier;
use failure::Fail;
use fixed_unsigned::RoundHalfUp;
use fixed_unsigned::types::{FixedUnsigned10, FixedScale10, FixedUnsigned26, FixedScale26};

use std::collections::HashMap;

pub struct Blockchain<'env> {
    pub(crate) env: &'env Environment,
    pub network_id: NetworkId,
    network_time: Arc<NetworkTime>,
    pub notifier: RwLock<Notifier<'env, BlockchainEvent>>,
    pub(crate) state: RwLock<BlockchainState<'env>>,
}

pub struct BlockchainState<'env> {
    accounts: Accounts<'env>,
    head_hash: Blake2bHash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushResult {
    Invalid(PushError),
    Orphan,
    Known,
    Extended,
    Rebranched,
    Forked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushError {
    InvalidBlock(BlockError),
    InvalidSuccessor,
    DuplicateTransaction,
    AccountsError(AccountError),
    InvalidFork,

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


#[derive(Debug, PartialEq, Eq)]
pub enum BlockchainEvent {
    Finalized,
    Extended(Blake2bHash),
    Rebranched(Vec<(Blake2bHash, Block)>, Vec<(Blake2bHash, Block)>),
}

impl<'env> Blockchain<'env> {

    pub fn new(env: &'env Environment, network_id: NetworkId, network_time: Arc<NetworkTime>) -> Result<Self, BlockchainError> { unimplemented!(); }

    fn load(env: &'env Environment, network_time: Arc<NetworkTime>, network_id: NetworkId, chain_store: ChainStore<'env>, head_hash: Blake2bHash) -> Result<Self, BlockchainError> { unimplemented!(); }

    fn init(env: &'env Environment, network_time: Arc<NetworkTime>, network_id: NetworkId, chain_store: ChainStore<'env>) -> Result<Self, BlockchainError> { unimplemented!(); }

    pub fn push_verify_dry(&self, block: &Block) -> Result<(), PushResult> { unimplemented!(); }

    pub fn push(&self, block: Block) -> PushResult { unimplemented!(); }

    fn extend(&self, block_hash: Blake2bHash, mut chain_info: ChainInfo, mut prev_info: ChainInfo) -> PushResult { unimplemented!(); }

    fn rebranch(&self, block_hash: Blake2bHash, chain_info: ChainInfo) -> PushResult { unimplemented!(); }

    pub fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool { unimplemented!() }

    pub fn get_block_at(&self, height: u32, include_body: bool) -> Option<Block> { unimplemented!() }

    pub fn get_block(&self, hash: &Blake2bHash, include_forks: bool, include_body: bool) -> Option<Block> { unimplemented!() }

    pub fn get_blocks(&self, start_block_hash: &Blake2bHash, count: u32, include_body: bool, direction: Direction) -> Vec<Block> { unimplemented!() }

    pub fn head_hash(&self) -> Blake2bHash { unimplemented!(); }

    pub fn height(&self) -> u32 { unimplemented!() }

    pub fn head(&self) -> MappedRwLockReadGuard<Block> { unimplemented!() }

    pub fn get_next_block_producer(&self, view_number: u32) -> (u16, Slot) { unimplemented!(); }

    pub fn get_next_validator_list(&self) -> Vec<Slot> { unimplemented!() }

    pub fn get_next_validator_set(&self) -> Vec<ValidatorSlots> { unimplemented!() }

    pub fn get_next_block_type(&self) -> BlockType { unimplemented!() }

    pub fn get_current_validator_by_idx(&self, validator_idx: u16) -> Option<ValidatorSlots> { unimplemented!() }

    pub fn is_in_current_epoch(&self, block_number: u32) -> bool { unimplemented!() }

    pub fn get_block_producer_at(&self, block_number: u32, view_number: u32) -> Option<(/*Index in slot list*/ u16, Slot)> { unimplemented!() }

    pub fn state(&self) -> RwLockReadGuard<BlockchainState<'env>> {
        self.state.read()
    }
}

pub struct Direction {}
pub struct ChainInfo {}
pub struct ChainStore<'env> { env: &'env Environment }
