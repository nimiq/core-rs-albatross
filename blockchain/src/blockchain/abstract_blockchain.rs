use futures::{future, stream::BoxStream, StreamExt};
use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainError, BlockchainEvent, ChainInfo, Direction, ForkEvent,
};
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::slots_allocation::{Validator, Validators};
use tokio_stream::wrappers::BroadcastStream;

use crate::Blockchain;

impl AbstractBlockchain for Blockchain {
    fn network_id(&self) -> NetworkId {
        self.network_id
    }

    fn now(&self) -> u64 {
        self.time.now()
    }

    fn head(&self) -> Block {
        self.state.main_chain.head.clone()
    }

    fn macro_head(&self) -> MacroBlock {
        self.state.macro_info.head.unwrap_macro_ref().clone()
    }

    fn election_head(&self) -> MacroBlock {
        self.state.election_head.clone()
    }

    fn block_number(&self) -> u32 {
        self.state.main_chain.head.block_number()
    }

    fn epoch_number(&self) -> u32 {
        self.state.main_chain.head.epoch_number()
    }

    fn accounts_complete(&self) -> bool {
        self.state.accounts.is_complete(None)
    }

    fn current_validators(&self) -> Option<Validators> {
        self.state.current_slots.clone()
    }

    fn previous_validators(&self) -> Option<Validators> {
        self.state.previous_slots.clone()
    }

    fn contains(&self, hash: &Blake2bHash, include_forks: bool) -> bool {
        match self.chain_store.get_chain_info(hash, false, None) {
            Ok(chain_info) => include_forks || chain_info.on_main_chain,
            Err(_) => false,
        }
    }

    fn get_block_at(&self, height: u32, include_body: bool) -> Result<Block, BlockchainError> {
        self.get_block_at(height, include_body, None)
    }

    fn get_block(&self, hash: &Blake2bHash, include_body: bool) -> Result<Block, BlockchainError> {
        self.get_block(hash, include_body, None)
    }

    fn get_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
    ) -> Result<Vec<Block>, BlockchainError> {
        self.get_blocks(start_block_hash, count, include_body, direction, None)
    }

    fn get_chain_info(
        &self,
        hash: &Blake2bHash,
        include_body: bool,
    ) -> Result<ChainInfo, BlockchainError> {
        self.get_chain_info(hash, include_body, None)
    }

    fn get_slot_owner_at(
        &self,
        block_number: u32,
        offset: u32,
    ) -> Result<(Validator, u16), BlockchainError> {
        self.get_slot_owner_at(block_number, offset, None)
    }

    fn get_macro_blocks(
        &self,
        start_block_hash: &Blake2bHash,
        count: u32,
        include_body: bool,
        direction: Direction,
        election_blocks_only: bool,
    ) -> Result<Vec<Block>, BlockchainError> {
        self.get_macro_blocks(
            start_block_hash,
            count,
            include_body,
            direction,
            election_blocks_only,
            None,
        )
    }

    fn notifier_as_stream(&self) -> BoxStream<'static, BlockchainEvent> {
        BroadcastStream::new(self.notifier.subscribe())
            .filter_map(|x| future::ready(x.ok()))
            .boxed()
    }

    fn fork_notifier_as_stream(&self) -> BoxStream<'static, ForkEvent> {
        BroadcastStream::new(self.fork_notifier.subscribe())
            .filter_map(|x| future::ready(x.ok()))
            .boxed()
    }

    fn get_tainted_config(&self) -> &nimiq_blockchain_interface::TaintedBlockchainConfig {
        &self.config.tainted_blockchain
    }
}
