extern crate nimiq_primitives as primitives;

use crate::chain_info::ChainInfo;
use crate::chain_store::ChainStore;
use genesis::NetworkInfo;
use nimiq_block_albatross::Block;
use nimiq_hash::Blake2bHash;
use primitives::networks::NetworkId;
use primitives::slot::Slots;
use std::sync::RwLock;

/// The Blockchain struct. It stores all information of the blockchain that is known to the Nano
/// nodes.
pub struct Blockchain {
    // The network ID. It determines if this is the mainnet or one of the testnets.
    pub network_id: NetworkId,
    // The hash of the head of the main chain.
    pub head_hash: Blake2bHash,
    // The hash of the last macro block.
    pub macro_head_hash: Blake2bHash,
    // The validator slots for the current epoch.
    pub current_slots: Option<Slots>,
    // The genesis block.
    pub genesis_block: Block,
    // The chain store is a database containing all of the chain infos in the current batch.
    pub chain_store: RwLock<ChainStore>,
    // The accounts tree.
    //pub accounts: Accounts,
}

/// Implements methods to start a Blockchain.
impl Blockchain {
    /// Creates a new blockchain from a given network ID.
    pub fn new(network_id: NetworkId) -> Self {
        let network_info = NetworkInfo::from_network_id(network_id);
        let genesis_block = network_info.genesis_block::<Block>();
        Self::with_genesis(network_id, genesis_block)
    }

    /// Creates a new blockchain with a given network ID and genesis block.
    pub fn with_genesis(network_id: NetworkId, genesis_block: Block) -> Self {
        let chain_info = ChainInfo {
            on_main_chain: true,
            main_chain_successor: None,
            predecessor: None,
        };

        let hash = genesis_block.hash();

        let number = genesis_block.block_number();

        let mut chain_store = ChainStore::new();

        chain_store.put_block_hash(number, hash.clone());

        chain_store.put_chain_info(hash.clone(), chain_info);

        chain_store.put_block(hash.clone(), genesis_block.clone());

        Blockchain {
            network_id,
            head_hash: hash.clone(),
            macro_head_hash: hash,
            current_slots: genesis_block.slots(),
            genesis_block,
            chain_store: RwLock::new(chain_store),
        }
    }
}
