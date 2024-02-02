use async_trait::async_trait;

use crate::types::{PolicyConstants, RPCResult};

#[nimiq_jsonrpc_derive::proxy(name = "PolicyProxy", rename_all = "camelCase")]
#[async_trait]
pub trait PolicyInterface {
    type Error;

    async fn get_policy_constants(&mut self) -> RPCResult<PolicyConstants, (), Self::Error>;

    async fn get_epoch_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    async fn get_epoch_index_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    async fn get_batch_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    async fn get_batch_index_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    async fn get_election_block_after(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    async fn get_election_block_before(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    async fn get_last_election_block(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    async fn is_election_block_at(&mut self, block_number: u32)
        -> RPCResult<bool, (), Self::Error>;

    async fn get_macro_block_after(&mut self, block_number: u32)
        -> RPCResult<u32, (), Self::Error>;

    async fn get_macro_block_before(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    async fn get_last_macro_block(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    async fn is_macro_block_at(&mut self, block_number: u32) -> RPCResult<bool, (), Self::Error>;

    async fn is_micro_block_at(&mut self, block_number: u32) -> RPCResult<bool, (), Self::Error>;

    async fn get_first_block_of(&mut self, epoch: u32) -> RPCResult<u32, (), Self::Error>;

    async fn get_first_block_of_batch(&mut self, batch: u32) -> RPCResult<u32, (), Self::Error>;

    async fn get_election_block_of(&mut self, epoch: u32) -> RPCResult<u32, (), Self::Error>;

    async fn get_macro_block_of(&mut self, batch: u32) -> RPCResult<u32, (), Self::Error>;

    async fn get_first_batch_of_epoch(
        &mut self,
        block_number: u32,
    ) -> RPCResult<bool, (), Self::Error>;

    async fn get_supply_at(
        &mut self,
        genesis_supply: u64,
        genesis_time: u64,
        current_time: u64,
    ) -> RPCResult<u64, (), Self::Error>;
}
