use async_trait::async_trait;

use crate::types::{PolicyConstants, RPCResult};

#[nimiq_jsonrpc_derive::proxy(name = "PolicyProxy", rename_all = "camelCase")]
#[async_trait]
pub trait PolicyInterface {
    type Error;

    /// Returns a bundle of policy constants.
    async fn get_policy_constants(&mut self) -> RPCResult<PolicyConstants, (), Self::Error>;

    /// Returns the epoch number at a given block number (height).
    async fn get_epoch_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the epoch index at a given block number. The epoch index is the number of a block relative
    /// to the epoch it is in. For example, the first block of any epoch always has an epoch index of 0.
    async fn get_epoch_index_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the batch number at a given `block_number` (height).
    async fn get_batch_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the batch index at a given block number. The batch index is the number of a block relative
    /// to the batch it is in. For example, the first block of any batch always has an batch index of 0.
    async fn get_batch_index_at(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the number (height) of the next election macro block after a given block number (height).
    async fn get_election_block_after(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns the number block (height) of the preceding election macro block before a given block number (height).
    /// If the given block number is an election macro block, it returns the election macro block before it.
    async fn get_election_block_before(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number (height) of the last election macro block at a given block number (height).
    /// If the given block number is an election macro block, then it returns that block number.
    async fn get_last_election_block(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns a boolean expressing if the block at a given block number (height) is an election macro block.
    async fn is_election_block_at(&mut self, block_number: u32)
        -> RPCResult<bool, (), Self::Error>;

    /// Returns the block number (height) of the next macro block after a given block number (height).
    async fn get_macro_block_after(&mut self, block_number: u32)
        -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number (height) of the preceding macro block before a given block number (height).
    /// If the given block number is a macro block, it returns the macro block before it.
    async fn get_macro_block_before(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns block the number (height) of the last macro block at a given block number (height).
    /// If the given block number is a macro block, then it returns that block number.
    async fn get_last_macro_block(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns a boolean expressing if the block at a given block number (height) is a macro block.
    async fn is_macro_block_at(&mut self, block_number: u32) -> RPCResult<bool, (), Self::Error>;

    /// Returns a boolean expressing if the block at a given block number (height) is a micro block.
    async fn is_micro_block_at(&mut self, block_number: u32) -> RPCResult<bool, (), Self::Error>;

    /// Returns the block number of the first block of the given epoch (which is always a micro block).
    async fn get_first_block_of(&mut self, epoch: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number of the first block of the given batch (which is always a micro block).
    async fn get_first_block_of_batch(&mut self, batch: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number of the election macro block of the given epoch (which is always the last block).
    async fn get_election_block_of(&mut self, epoch: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns the block number of the macro block (checkpoint or election) of the given batch (which
    /// is always the last block).
    async fn get_macro_block_of(&mut self, batch: u32) -> RPCResult<u32, (), Self::Error>;

    /// Returns a boolean expressing if the batch at a given block number (height) is the first batch
    /// of the epoch.
    async fn get_first_batch_of_epoch(
        &mut self,
        block_number: u32,
    ) -> RPCResult<bool, (), Self::Error>;

    /// Returns the first block after the reporting window of a given block number has ended.
    async fn get_block_after_reporting_window(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns the first block after the jail period of a given block number has ended.
    async fn get_block_after_jail(&mut self, block_number: u32) -> RPCResult<u32, (), Self::Error>;

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
    ) -> RPCResult<u64, (), Self::Error>;
}
