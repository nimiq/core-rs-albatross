use nimiq_block_albatross::{Block, MacroBlock};
use nimiq_blockchain_albatross::{AbstractBlockchain, ChainInfo};
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::slots::Validators;

use crate::blockchain::NanoBlockchain;

impl AbstractBlockchain for NanoBlockchain {
    fn network_id(&self) -> NetworkId {
        self.network_id
    }

    fn now(&self) -> u64 {
        self.time.now()
    }

    fn head(&self) -> Block {
        self.head.clone()
    }

    fn macro_head(&self) -> MacroBlock {
        self.macro_head.clone()
    }

    fn election_head(&self) -> MacroBlock {
        self.election_head.clone()
    }

    fn current_validators(&self) -> Option<Validators> {
        self.current_slots.clone()
    }

    fn previous_validators(&self) -> Option<Validators> {
        unreachable!()
    }

    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool {
        match self.chain_store.read().unwrap().get_chain_info(hash) {
            Some(chain_info) => include_forks || chain_info.on_main_chain,
            None => false,
        }
    }

    fn get_block_at(&self, height: u32, _include_body: bool) -> Option<Block> {
        self.chain_store
            .read()
            .unwrap()
            .get_chain_info_at(height)
            .map(|chain_info| chain_info.head)
    }

    fn get_block(&self, hash: &Blake2bHash, _include_body: bool) -> Option<Block> {
        self.chain_store
            .read()
            .unwrap()
            .get_chain_info(hash)
            .map(|chain_info| chain_info.head.clone())
    }

    fn get_chain_info(&self, hash: &Blake2bHash, _include_body: bool) -> Option<ChainInfo> {
        self.chain_store
            .read()
            .unwrap()
            .get_chain_info(hash)
            .cloned()
    }
}
