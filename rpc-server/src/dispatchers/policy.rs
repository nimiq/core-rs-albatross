use async_trait::async_trait;

use nimiq_primitives::policy;
use nimiq_rpc_interface::policy::PolicyInterface;
use nimiq_rpc_interface::types::PolicyConstants;

use crate::error::Error;

pub struct PolicyDispatcher {}

impl PolicyDispatcher {
    pub fn new() -> Self {
        PolicyDispatcher {}
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl PolicyInterface for PolicyDispatcher {
    type Error = Error;

    /// Returns a bundle of policy constants
    async fn get_policy_constants(&mut self) -> Result<PolicyConstants, Self::Error> {
        Ok(PolicyConstants {
            staking_contract_address: policy::STAKING_CONTRACT_ADDRESS.to_string(),
            coinbase_address: policy::COINBASE_ADDRESS.to_string(),
            transaction_validity_window: policy::TRANSACTION_VALIDITY_WINDOW,
            max_size_micro_body: policy::MAX_SIZE_MICRO_BODY,
            version: policy::VERSION,
            slots: policy::SLOTS,
            blocks_per_batch: policy::BLOCKS_PER_BATCH,
            batches_per_epoch: policy::BATCHES_PER_EPOCH,
            validator_deposit: policy::VALIDATOR_DEPOSIT,
            total_supply: policy::TOTAL_SUPPLY,
        })
    }

    /// Returns the epoch number at a given block number (height).
    async fn get_epoch_at(&mut self, block_number: u32) -> Result<u32, Self::Error> {
        Ok(policy::epoch_at(block_number))
    }

    /// Returns the epoch index at a given block number. The epoch index is the number of a block relative
    /// to the the epoch it is in. For example, the first block of any epoch always has an epoch index of 0.
    async fn get_epoch_index_at(&mut self, block_number: u32) -> Result<u32, Self::Error> {
        Ok(policy::epoch_index_at(block_number))
    }

    /// Returns the batch number at a given `block_number` (height)
    async fn get_batch_at(&mut self, block_number: u32) -> Result<u32, Self::Error> {
        Ok(policy::batch_at(block_number))
    }

    /// Returns the batch index at a given block number. The batch index is the number of a block relative
    /// to the the batch it is in. For example, the first block of any batch always has an batch index of 0.
    async fn get_batch_index_at(&mut self, block_number: u32) -> Result<u32, Self::Error> {
        Ok(policy::batch_index_at(block_number))
    }

    /// Returns the number (height) of the next election macro block after a given block number (height).
    async fn get_election_block_after(&mut self, block_number: u32) -> Result<u32, Self::Error> {
        Ok(policy::election_block_after(block_number))
    }

    /// Returns the number (height) of the preceding election macro block before a given block number (height).
    /// If the given block number is an  election macro block, it returns the election macro block before it.
    async fn get_election_block_before(&mut self, block_number: u32) -> Result<u32, Self::Error> {
        if block_number == 0 {
            return Err(Error::BlockNumberNotZero);
        }

        Ok(policy::election_block_before(block_number))
    }

    /// Returns the number (height) of the last election macro block at a given block number (height).
    /// If the given block number is an election macro block, then it returns that block number.
    async fn get_last_election_block(&mut self, block_number: u32) -> Result<u32, Self::Error> {
        Ok(policy::last_election_block(block_number))
    }

    /// Returns a boolean expressing if the block at a given block number (height) is an election macro block.
    async fn get_is_election_block_at(&mut self, block_number: u32) -> Result<bool, Self::Error> {
        Ok(policy::is_election_block_at(block_number))
    }

    /// Returns the number (height) of the next macro block after a given block number (height).
    async fn get_macro_block_after(&mut self, block_number: u32) -> Result<u32, Self::Error> {
        Ok(policy::macro_block_after(block_number))
    }

    /// Returns the number (height) of the preceding macro block before a given block number (height).
    /// If the given block number is a macro block, it returns the macro block before it.
    async fn get_macro_block_before(&mut self, block_number: u32) -> Result<u32, Self::Error> {
        if block_number == 0 {
            return Err(Error::BlockNumberNotZero);
        }

        Ok(policy::macro_block_before(block_number))
    }

    /// Returns the number (height) of the last macro block at a given block number (height).
    /// If the given block number is a macro block, then it returns that block number.
    async fn get_last_macro_block(&mut self, block_number: u32) -> Result<u32, Self::Error> {
        Ok(policy::last_macro_block(block_number))
    }

    /// Returns a boolean expressing if the block at a given block number (height) is a macro block.
    async fn get_is_macro_block_at(&mut self, block_number: u32) -> Result<bool, Self::Error> {
        Ok(policy::is_macro_block_at(block_number))
    }

    /// Returns a boolean expressing if the block at a given block number (height) is a micro block.
    async fn get_is_micro_block_at(&mut self, block_number: u32) -> Result<bool, Self::Error> {
        Ok(policy::is_micro_block_at(block_number))
    }

    /// Returns the block number of the first block of the given epoch (which is always a micro block).
    async fn get_first_block_of(&mut self, epoch: u32) -> Result<u32, Self::Error> {
        if epoch == 0 {
            return Err(Error::EpochNumberNotZero);
        }

        Ok(policy::first_block_of(epoch))
    }

    ///  Returns the block number of the first block of the given batch (which is always a micro block).
    async fn get_first_block_of_batch(&mut self, batch: u32) -> Result<u32, Self::Error> {
        if batch == 0 {
            return Err(Error::BatchNumberNotZero);
        }

        Ok(policy::first_block_of_batch(batch))
    }

    /// Returns the block number of the election macro block of the given epoch (which is always the last block).
    async fn get_election_block_of(&mut self, epoch: u32) -> Result<u32, Self::Error> {
        Ok(policy::election_block_of(epoch))
    }

    /// Returns the block number of the macro block (checkpoint or election) of the given batch (which
    /// is always the last block).
    async fn get_macro_block_of(&mut self, batch: u32) -> Result<u32, Self::Error> {
        Ok(policy::macro_block_of(batch))
    }

    /// Returns a boolean expressing if the batch at a given block number (height) is the first batch
    /// of the epoch.
    async fn get_first_batch_of_epoch(&mut self, block_number: u32) -> Result<bool, Self::Error> {
        Ok(policy::first_batch_of_epoch(block_number))
    }

    /// Returns the supply at a given time (as Unix time) in Lunas (1 NIM = 100,000 Lunas). It is
    /// calculated using the following formula:
    /// Supply (t) = Genesis_supply + Initial_supply_velocity / Supply_decay * (1 - e^(- Supply_decay * t))
    /// Where e is the exponential function, t is the time in milliseconds since the genesis block and
    /// Genesis_supply is the supply at the genesis of the Nimiq 2.0 chain.
    async fn get_supply_at(
        &mut self,
        genesis_supply: u64,
        genesis_time: u64,
        current_time: u64,
    ) -> Result<u64, Self::Error> {
        Ok(policy::supply_at(
            genesis_supply,
            genesis_time,
            current_time,
        ))
    }
}
