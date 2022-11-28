use std::sync::Arc;

use futures::stream::BoxStream;
use parking_lot::{RwLock, RwLockReadGuard};

use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain::ChainInfo;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, BlockchainEvent, Direction};
use nimiq_database::Transaction;
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2bHash;
use nimiq_light_blockchain::LightBlockchain;
use nimiq_primitives::slots::{Validator, Validators};

macro_rules! gen_blockchain_match {
    ($self: ident, $t: ident, $f: ident $(, $arg:expr )*) => {
        match $self {
            $t::Full(ref blockchain) => AbstractBlockchain::$f(&***blockchain, $( $arg ),*),
            $t::Light(ref nano_blockchain) => AbstractBlockchain::$f(&***nano_blockchain, $( $arg ),*),
        }
    };
}

/// The `BlockchainProxy` is our abstraction over multiple types of blockchains.
pub enum BlockchainProxy {
    /// Full Blockchain, stores the full history, transactions, and full blocks.
    Full(Arc<RwLock<Blockchain>>),
    /// Light Blockchain, stores only ZKPs, election macro blocks, and block header and their justification.
    Light(Arc<RwLock<LightBlockchain>>),
}

impl Clone for BlockchainProxy {
    fn clone(&self) -> Self {
        match self {
            Self::Full(blockchain) => Self::Full(Arc::clone(blockchain)),
            Self::Light(nano_blockchain) => Self::Light(Arc::clone(nano_blockchain)),
        }
    }
}

impl From<Arc<RwLock<Blockchain>>> for BlockchainProxy {
    fn from(blockchain: Arc<RwLock<Blockchain>>) -> Self {
        Self::Full(blockchain)
    }
}

impl From<Arc<RwLock<LightBlockchain>>> for BlockchainProxy {
    fn from(nano_blockchain: Arc<RwLock<LightBlockchain>>) -> Self {
        Self::Light(nano_blockchain)
    }
}

impl<'a> From<&'a Arc<RwLock<Blockchain>>> for BlockchainProxy {
    fn from(blockchain: &'a Arc<RwLock<Blockchain>>) -> Self {
        Self::Full(Arc::clone(blockchain))
    }
}

impl<'a> From<&'a Arc<RwLock<LightBlockchain>>> for BlockchainProxy {
    fn from(nano_blockchain: &'a Arc<RwLock<LightBlockchain>>) -> Self {
        Self::Light(Arc::clone(nano_blockchain))
    }
}

impl BlockchainProxy {
    /// Returns a wrapper/proxy around a read locked blockchain.
    /// The `BlockchainReadProxy` implements `AbstractBlockchain` and allows to access common blockchain functions.
    pub fn read(&self) -> BlockchainReadProxy {
        match self {
            BlockchainProxy::Full(blockchain) => {
                BlockchainReadProxy::Full(Arc::new(blockchain.read()))
            }
            BlockchainProxy::Light(nano_blockchain) => {
                BlockchainReadProxy::Light(Arc::new(nano_blockchain.read()))
            }
        }
    }
}

/// The `BlockchainReadProxy` implements `AbstractBlockchain` and allows to access common blockchain functions.
/// It is a wrapper around read locked versions of our blockchain types.
pub enum BlockchainReadProxy<'a> {
    // Read locked access to a Full Blockchain
    Full(Arc<RwLockReadGuard<'a, Blockchain>>),
    // Read locked access to a Light Blockchain
    Light(Arc<RwLockReadGuard<'a, LightBlockchain>>),
}

impl<'a> AbstractBlockchain for BlockchainReadProxy<'a> {
    fn network_id(&self) -> NetworkId {
        gen_blockchain_match!(self, BlockchainReadProxy, network_id)
    }

    fn now(&self) -> u64 {
        gen_blockchain_match!(self, BlockchainReadProxy, now)
    }

    fn head(&self) -> Block {
        gen_blockchain_match!(self, BlockchainReadProxy, head)
    }

    fn macro_head(&self) -> MacroBlock {
        gen_blockchain_match!(self, BlockchainReadProxy, macro_head)
    }

    fn election_head(&self) -> MacroBlock {
        gen_blockchain_match!(self, BlockchainReadProxy, election_head)
    }

    fn current_validators(&self) -> Option<Validators> {
        gen_blockchain_match!(self, BlockchainReadProxy, current_validators)
    }

    fn previous_validators(&self) -> Option<Validators> {
        gen_blockchain_match!(self, BlockchainReadProxy, previous_validators)
    }

    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool {
        gen_blockchain_match!(self, BlockchainReadProxy, contains, hash, include_forks)
    }

    fn get_block_at(
        &self,
        height: u32,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Block> {
        gen_blockchain_match!(
            self,
            BlockchainReadProxy,
            get_block_at,
            height,
            include_body,
            txn_option
        )
    }

    fn get_block(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Block> {
        gen_blockchain_match!(
            self,
            BlockchainReadProxy,
            get_block,
            hash,
            include_body,
            txn_option
        )
    }

    fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: nimiq_blockchain::Direction,
        txn_option: Option<&Transaction>,
    ) -> Vec<Block> {
        gen_blockchain_match!(
            self,
            BlockchainReadProxy,
            get_blocks,
            start_block_hash,
            count,
            include_body,
            direction,
            txn_option
        )
    }

    fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<ChainInfo> {
        gen_blockchain_match!(
            self,
            BlockchainReadProxy,
            get_chain_info,
            hash,
            include_body,
            txn_option
        )
    }

    fn get_slot_owner_at(
        &self,
        block_number: u32,
        offset: u32,
        txn_option: Option<&Transaction>,
    ) -> Option<(Validator, u16)> {
        gen_blockchain_match!(
            self,
            BlockchainReadProxy,
            get_slot_owner_at,
            block_number,
            offset,
            txn_option
        )
    }

    fn get_macro_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        election_blocks_only: bool,
        txn_option: Option<&Transaction>,
    ) -> Option<Vec<Block>> {
        gen_blockchain_match!(
            self,
            BlockchainReadProxy,
            get_macro_blocks,
            start_block_hash,
            count,
            include_body,
            direction,
            election_blocks_only,
            txn_option
        )
    }

    fn notifier_as_stream(&self) -> BoxStream<'static, BlockchainEvent> {
        gen_blockchain_match!(self, BlockchainReadProxy, notifier_as_stream)
    }
}
