use futures::stream::BoxStream;
use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain::{AbstractBlockchain, BlockchainEvent, ChainInfo};
use nimiq_database::Transaction;
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::slots::{Validator, Validators};

use crate::blockchain::LightBlockchain;

/// Implements several basic methods for blockchains.
impl AbstractBlockchain for LightBlockchain {
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
        self.current_validators.clone()
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

    fn get_block_at(
        &self,
        height: u32,
        _include_body: bool,
        _txn_option: Option<&Transaction>,
    ) -> Option<Block> {
        self.chain_store
            .read()
            .unwrap()
            .get_chain_info_at(height)
            .map(|chain_info| chain_info.head)
    }

    fn get_block(
        &self,
        hash: &Blake2bHash,
        _include_body: bool,
        _txn_option: Option<&Transaction>,
    ) -> Option<Block> {
        self.chain_store
            .read()
            .unwrap()
            .get_chain_info(hash)
            .map(|chain_info| chain_info.head.clone())
    }

    fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        _include_body: bool,
        _txn_option: Option<&Transaction>,
    ) -> Option<ChainInfo> {
        self.chain_store
            .read()
            .unwrap()
            .get_chain_info(hash)
            .cloned()
    }

    fn get_slot_owner_at(
        &self,
        _block_number: u32,
        _offset: u32,
        _txn_option: Option<&Transaction>,
    ) -> Option<(Validator, u16)> {
        todo!() // IPTODO
    }

    fn notifier_as_stream(&self) -> BoxStream<'static, BlockchainEvent> {
        todo!() // IPTODO
    }

    fn get_blocks(
        &self,
        _start_block_hash: &Blake2bHash,
        _count: u32,
        _include_body: bool,
        _direction: nimiq_blockchain::Direction,
        _txn_option: Option<&Transaction>,
    ) -> Vec<Block> {
        todo!() // IPTODO
    }
}
