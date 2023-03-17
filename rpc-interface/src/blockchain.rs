use async_trait::async_trait;
use futures::stream::BoxStream;

use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;

use crate::types::{
    Account, Block, BlockLog, BlockchainState, ExecutedTransaction, Inherent, LogType, ParkedSet,
    RPCData, RPCResult, SlashedSlots, Slot, Staker, Validator,
};

#[nimiq_jsonrpc_derive::proxy(name = "BlockchainProxy", rename_all = "camelCase")]
#[async_trait]
pub trait BlockchainInterface {
    type Error;

    async fn get_block_number(&mut self) -> RPCResult<u32, (), Self::Error>;

    async fn get_batch_number(&mut self) -> RPCResult<u32, (), Self::Error>;

    async fn get_epoch_number(&mut self) -> RPCResult<u32, (), Self::Error>;

    async fn get_block_by_hash(
        &mut self,
        hash: Blake2bHash,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    async fn get_block_by_number(
        &mut self,
        block_number: u32,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    async fn get_latest_block(
        &mut self,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    async fn get_slot_at(
        &mut self,
        block_number: u32,
        offset_opt: Option<u32>,
    ) -> RPCResult<Slot, BlockchainState, Self::Error>;

    async fn get_transaction_by_hash(
        &mut self,
        hash: Blake2bHash,
    ) -> RPCResult<ExecutedTransaction, (), Self::Error>;

    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    async fn get_inherents_by_block_number(
        &mut self,
        block_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error>;

    async fn get_transactions_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    async fn get_inherents_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error>;

    // TODO: includes reward txs
    async fn get_transaction_hashes_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> RPCResult<Vec<Blake2bHash>, (), Self::Error>;

    async fn get_transactions_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    async fn get_account_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Account, BlockchainState, Self::Error>;

    async fn get_active_validators(
        &mut self,
    ) -> RPCResult<Vec<Validator>, BlockchainState, Self::Error>;

    async fn get_current_slashed_slots(
        &mut self,
    ) -> RPCResult<SlashedSlots, BlockchainState, Self::Error>;

    async fn get_previous_slashed_slots(
        &mut self,
    ) -> RPCResult<SlashedSlots, BlockchainState, Self::Error>;

    async fn get_parked_validators(&mut self)
        -> RPCResult<ParkedSet, BlockchainState, Self::Error>;

    async fn get_validator_by_address(
        &mut self,
        address: Address,
        include_stakers: Option<bool>,
    ) -> RPCResult<Validator, BlockchainState, Self::Error>;

    async fn get_staker_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Staker, BlockchainState, Self::Error>;

    #[stream]
    async fn subscribe_for_head_block(
        &mut self,
        include_body: Option<bool>,
    ) -> Result<BoxStream<'static, RPCData<Block, ()>>, Self::Error>;

    #[stream]
    async fn subscribe_for_head_block_hash(
        &mut self,
    ) -> Result<BoxStream<'static, RPCData<Blake2bHash, ()>>, Self::Error>;

    #[stream]
    async fn subscribe_for_validator_election_by_address(
        &mut self,
        address: Address,
    ) -> Result<BoxStream<'static, RPCData<Validator, BlockchainState>>, Self::Error>;

    #[stream]
    async fn subscribe_for_logs_by_addresses_and_types(
        &mut self,
        addresses: Vec<Address>,
        log_types: Vec<LogType>,
    ) -> Result<BoxStream<'static, RPCData<BlockLog, BlockchainState>>, Self::Error>;
}
