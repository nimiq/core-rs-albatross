use std::collections::HashMap;

use async_trait::async_trait;
use futures::stream::BoxStream;

use nimiq_hash::Blake3Hash;
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;

use crate::types::{
    Account, Block, Inherent, ParkedSet, SlashedSlots, Slot, Staker, Transaction, Validator,
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
        hash: Blake3Hash,
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
        view_number: Option<u32>,
    ) -> Result<Slot, Self::Error>;

    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> Result<Vec<Transaction>, Self::Error>;

    async fn get_inherents_by_block_number(
        &mut self,
        block_number: u32,
    ) -> Result<Vec<Inherent>, Self::Error>;

    async fn get_transactions_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> Result<Vec<Transaction>, Self::Error>;

    async fn get_inherents_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> Result<Vec<Inherent>, Self::Error>;

    // TODO: includes reward txs
    async fn get_transaction_hashes_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> Result<Vec<Blake3Hash>, Self::Error>;

    async fn get_transactions_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> Result<Vec<Transaction>, Self::Error>;

    async fn get_account_by_address(&mut self, address: Address) -> Result<Account, Self::Error>;

    async fn get_active_validators(&mut self) -> Result<HashMap<Address, Coin>, Self::Error>;

    async fn get_current_slashed_slots(&mut self) -> Result<SlashedSlots, Self::Error>;

    async fn get_previous_slashed_slots(&mut self) -> Result<SlashedSlots, Self::Error>;

    async fn get_parked_validators(&mut self) -> Result<ParkedSet, Self::Error>;

    async fn get_validator_by_address(
        &mut self,
        address: Address,
        include_stakers: Option<bool>,
    ) -> Result<Validator, Self::Error>;

    async fn get_staker_by_address(&mut self, address: Address) -> Result<Staker, Self::Error>;

    #[stream]
    async fn head_subscribe(&mut self) -> Result<BoxStream<'static, Blake3Hash>, Self::Error>;
}
