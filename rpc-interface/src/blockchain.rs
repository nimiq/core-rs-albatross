use async_trait::async_trait;
use futures::stream::BoxStream;

use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;

use crate::types::{
    Account, Block, BlockLog, ExecutedTransaction, Inherent, LogType, ParkedSet, RPCResult,
    SlashedSlots, Slot, Staker, Validator,
};

#[nimiq_jsonrpc_derive::proxy(name = "BlockchainProxy", rename_all = "camelCase")]
#[async_trait]
pub trait BlockchainInterface {
    type Error;

    async fn get_block_number(&mut self) -> Result<u32, Self::Error>;

    async fn get_batch_number(&mut self) -> Result<u32, Self::Error>;

    async fn get_epoch_number(&mut self) -> Result<u32, Self::Error>;

    async fn get_block_by_hash(
        &mut self,
        hash: Blake2bHash,
        include_transactions: Option<bool>,
    ) -> Result<Block, Self::Error>;

    async fn get_block_by_number(
        &mut self,
        block_number: u32,
        include_transactions: Option<bool>,
    ) -> Result<Block, Self::Error>;

    async fn get_latest_block(
        &mut self,
        include_transactions: Option<bool>,
    ) -> Result<Block, Self::Error>;

    async fn get_slot_at(
        &mut self,
        block_number: u32,
        offset_opt: Option<u32>,
    ) -> Result<RPCResult<Slot>, Self::Error>;

    async fn get_transaction_by_hash(
        &mut self,
        hash: Blake2bHash,
    ) -> Result<ExecutedTransaction, Self::Error>;

    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> Result<Vec<ExecutedTransaction>, Self::Error>;

    async fn get_inherents_by_block_number(
        &mut self,
        block_number: u32,
    ) -> Result<Vec<Inherent>, Self::Error>;

    async fn get_transactions_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> Result<Vec<ExecutedTransaction>, Self::Error>;

    async fn get_inherents_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> Result<Vec<Inherent>, Self::Error>;

    // TODO: includes reward txs
    async fn get_transaction_hashes_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> Result<Vec<Blake2bHash>, Self::Error>;

    async fn get_transactions_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> Result<Vec<ExecutedTransaction>, Self::Error>;

    async fn get_account_by_address(
        &mut self,
        address: Address,
    ) -> Result<RPCResult<Account>, Self::Error>;

    async fn get_active_validators(&mut self) -> Result<RPCResult<Vec<Validator>>, Self::Error>;

    async fn get_current_slashed_slots(&mut self) -> Result<RPCResult<SlashedSlots>, Self::Error>;

    async fn get_previous_slashed_slots(&mut self) -> Result<RPCResult<SlashedSlots>, Self::Error>;

    async fn get_parked_validators(&mut self) -> Result<RPCResult<ParkedSet>, Self::Error>;

    async fn get_validator_by_address(
        &mut self,
        address: Address,
        include_stakers: Option<bool>,
    ) -> Result<RPCResult<Validator>, Self::Error>;

    async fn get_staker_by_address(
        &mut self,
        address: Address,
    ) -> Result<RPCResult<Staker>, Self::Error>;

    #[stream]
    async fn subscribe_for_head_block(
        &mut self,
        include_transactions: Option<bool>,
    ) -> Result<BoxStream<'static, Block>, Self::Error>;

    #[stream]
    async fn subscribe_for_head_block_hash(
        &mut self,
    ) -> Result<BoxStream<'static, Blake2bHash>, Self::Error>;

    #[stream]
    async fn subscribe_for_validator_election_by_address(
        &mut self,
        address: Address,
    ) -> Result<BoxStream<'static, RPCResult<Validator>>, Self::Error>;

    #[stream]
    async fn subscribe_for_logs_by_addresses_and_types(
        &mut self,
        addresses: Vec<Address>,
        log_types: Vec<LogType>,
    ) -> Result<BoxStream<'static, RPCResult<BlockLog>>, Self::Error>;
}
