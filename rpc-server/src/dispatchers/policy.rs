use async_trait::async_trait;
use nimiq_primitives::policy::Policy;
use nimiq_rpc_interface::{
    policy::PolicyInterface,
    types::{PolicyConstants, RPCResult},
};

use crate::error::Error;

pub struct PolicyDispatcher {}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl PolicyInterface for PolicyDispatcher {
    type Error = Error;

    async fn get_policy_constants(&mut self) -> RPCResult<PolicyConstants, (), Self::Error> {
        Ok(PolicyConstants {
            staking_contract_address: Policy::STAKING_CONTRACT_ADDRESS.to_string(),
            coinbase_address: Policy::COINBASE_ADDRESS.to_string(),
            transaction_validity_window: Policy::transaction_validity_window_blocks(),
            max_size_micro_body: Policy::MAX_SIZE_MICRO_BODY,
            version: Policy::VERSION,
            slots: Policy::SLOTS,
            blocks_per_batch: Policy::blocks_per_batch(),
            batches_per_epoch: Policy::batches_per_epoch(),
            blocks_per_epoch: Policy::blocks_per_epoch(),
            validator_deposit: Policy::VALIDATOR_DEPOSIT,
            minimum_stake: Policy::MINIMUM_STAKE,
            total_supply: Policy::TOTAL_SUPPLY,
            block_separation_time: Policy::BLOCK_SEPARATION_TIME,
            jail_epochs: Policy::JAIL_EPOCHS,
            genesis_block_number: Policy::genesis_block_number(),
        }
        .into())
    }

    async fn get_epoch_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::epoch_at(block_number).into())
    }

    async fn get_epoch_index_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::epoch_index_at(block_number).into())
    }

    async fn get_batch_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::batch_at(block_number).into())
    }

    async fn get_batch_index_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::batch_index_at(block_number).into())
    }

    async fn get_election_block_after(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::election_block_after(block_number).into())
    }

    async fn get_election_block_before(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error> {
        if block_number < Policy::genesis_block_number() {
            return Err(Error::BlockNumberBeforeGenesis);
        }
        Ok(Policy::election_block_before(block_number).into())
    }

    async fn get_last_election_block(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error> {
        if block_number < Policy::genesis_block_number() {
            return Err(Error::BlockNumberBeforeGenesis);
        }
        Ok(Policy::last_election_block(block_number).into())
    }

    async fn is_election_block_at(
        &mut self,
        block_number: u32,
    ) -> RPCResult<bool, (), Self::Error> {
        Ok(Policy::is_election_block_at(block_number).into())
    }

    async fn get_macro_block_after(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::macro_block_after(block_number).into())
    }

    async fn get_macro_block_before(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error> {
        if block_number < Policy::genesis_block_number() {
            return Err(Error::BlockNumberBeforeGenesis);
        }
        Ok(Policy::macro_block_before(block_number).into())
    }

    async fn get_last_macro_block(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error> {
        if block_number < Policy::genesis_block_number() {
            return Err(Error::BlockNumberBeforeGenesis);
        }
        Ok(Policy::last_macro_block(block_number).into())
    }

    async fn is_macro_block_at(&mut self, block_number: u32) -> RPCResult<bool, (), Self::Error> {
        Ok(Policy::is_macro_block_at(block_number).into())
    }

    async fn is_micro_block_at(&mut self, block_number: u32) -> RPCResult<bool, (), Self::Error> {
        Ok(Policy::is_micro_block_at(block_number).into())
    }

    async fn get_first_block_of(&mut self, epoch: u32) -> RPCResult<u32, (), Self::Error> {
        match Policy::first_block_of(epoch) {
            Some(block_number) => Ok(block_number.into()),
            None => Err(Error::InvalidArgument("epoch".to_string())),
        }
    }

    async fn get_first_block_of_batch(&mut self, batch: u32) -> RPCResult<u32, (), Self::Error> {
        match Policy::first_block_of_batch(batch) {
            Some(block_number) => Ok(block_number.into()),
            None => Err(Error::InvalidArgument("batch".to_string())),
        }
    }

    async fn get_election_block_of(&mut self, epoch: u32) -> RPCResult<u32, (), Self::Error> {
        match Policy::election_block_of(epoch) {
            Some(block_number) => Ok(block_number.into()),
            None => Err(Error::InvalidArgument("epoch".to_string())),
        }
    }

    async fn get_macro_block_of(&mut self, batch: u32) -> RPCResult<u32, (), Self::Error> {
        match Policy::macro_block_of(batch) {
            Some(block_number) => Ok(block_number.into()),
            None => Err(Error::InvalidArgument("batch".to_string())),
        }
    }

    async fn get_first_batch_of_epoch(
        &mut self,
        block_number: u32,
    ) -> RPCResult<bool, (), Self::Error> {
        Ok(Policy::first_batch_of_epoch(block_number).into())
    }

    async fn get_block_after_reporting_window(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::block_after_reporting_window(block_number).into())
    }

    async fn get_block_after_jail(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error> {
        Ok(Policy::block_after_jail(block_number).into())
    }

    async fn get_supply_at(
        &mut self,
        genesis_supply: u64,
        genesis_time: u64,
        current_time: u64,
    ) -> RPCResult<u64, (), Self::Error> {
        if Policy::POW_GENESIS_TIMESTAMP > current_time {
            log::error!(
                current_time,
                pow_genesis_ts = Policy::POW_GENESIS_TIMESTAMP,
                "The supplied time must be equal or greater than the PoW genesis timestamp"
            );
            return Err(Error::InvalidArgument(current_time.to_string()));
        }

        Ok(Policy::supply_at(genesis_supply, genesis_time, current_time).into())
    }
}
