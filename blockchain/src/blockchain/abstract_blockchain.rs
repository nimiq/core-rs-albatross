use futures::{future, stream::BoxStream, StreamExt};
use nimiq_block::{Block, MacroBlock};
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainError, BlockchainEvent, ChainInfo, Direction, ForkEvent,
};
use nimiq_genesis::NetworkId;
use nimiq_hash::Blake2bHash;
use nimiq_primitives::{
    policy::Policy,
    slots_allocation::{Slot, Validators},
};
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

    fn batch_number(&self) -> u32 {
        self.state.main_chain.head.batch_number()
    }

    fn epoch_number(&self) -> u32 {
        self.state.main_chain.head.epoch_number()
    }

    fn can_enforce_validity_window(&self) -> bool {
        // If we are at the genesis block, we can enforce the validity window
        if self.block_number() == Policy::genesis_block_number() {
            true
        } else {
            // We can enforce the history root when our history store root equals our head one.
            *self.head().history_root()
                == self
                    .history_store
                    .get_history_tree_root(self.block_number(), None)
                    .unwrap()
        }
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

    fn get_genesis_hash(&self) -> Blake2bHash {
        self.genesis_hash.clone()
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

    fn get_proposer_at(&self, block_number: u32, offset: u32) -> Result<Slot, BlockchainError> {
        self.get_proposer_at(block_number, offset, None)
    }

    fn get_proposer_of(&self, block_hash: &Blake2bHash) -> Result<Slot, BlockchainError> {
        self.get_proposer_of(block_hash, None)
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
}
