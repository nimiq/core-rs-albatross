use std::sync::{Arc, RwLock};

use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain::ChainInfo;
use nimiq_genesis::NetworkInfo;
use nimiq_primitives::networks::NetworkId;
use nimiq_primitives::slots::Validators;
use nimiq_utils::time::OffsetTime;

use crate::chain_store::ChainStore;

/// The Blockchain struct. It stores all information of the blockchain that is known to the Nano
/// nodes.
pub struct NanoBlockchain {
    // The network ID. It determines if this is the mainnet or one of the testnets.
    pub network_id: NetworkId,
    // The OffsetTime struct. It allows us to query the current time.
    pub time: Arc<OffsetTime>,
    // The head of the main chain.
    pub head: Block,
    // The last macro block.
    pub macro_head: MacroBlock,
    // The last election block.
    pub election_head: MacroBlock,
    // The validators for the current epoch.
    pub current_validators: Option<Validators>,
    // The genesis block.
    pub genesis_block: Block,
    // The chain store is a database containing all of the chain infos in the current batch.
    pub chain_store: RwLock<ChainStore>,
}

/// Implements methods to start a Blockchain.
impl NanoBlockchain {
    /// Creates a new blockchain from a given network ID.
    pub fn new(network_id: NetworkId) -> Self {
        let network_info = NetworkInfo::from_network_id(network_id);
        let genesis_block = network_info.genesis_block::<Block>();
        Self::with_genesis(network_id, genesis_block)
    }

    /// Creates a new blockchain with a given network ID and genesis block.
    pub fn with_genesis(network_id: NetworkId, genesis_block: Block) -> Self {
        let time = Arc::new(OffsetTime::new());

        let chain_info = ChainInfo::new(genesis_block.clone(), true);

        let mut chain_store = ChainStore::default();

        chain_store.put_chain_info(chain_info);

        NanoBlockchain {
            network_id,
            time,
            head: genesis_block.clone(),
            macro_head: genesis_block.clone().unwrap_macro(),
            election_head: genesis_block.clone().unwrap_macro(),
            current_validators: genesis_block.validators(),
            genesis_block,
            chain_store: RwLock::new(chain_store),
        }
    }
}
