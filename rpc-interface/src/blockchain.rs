use async_trait::async_trait;
use futures::stream::BoxStream;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;

use crate::types::{
    Account, Block, BlockLog, BlockchainState, ExecutedTransaction, Inherent, LogType,
    PenalizedSlots, RPCData, RPCResult, Slot, Staker, Validator,
};

#[nimiq_jsonrpc_derive::proxy(name = "BlockchainProxy", rename_all = "camelCase")]
#[async_trait]
pub trait BlockchainInterface {
    type Error;

    /// Returns the block number for the current head.
    async fn get_block_number(&mut self) -> RPCResult<u32, (), Self::Error>;

    /// Returns the batch number for the current head.
    async fn get_batch_number(&mut self) -> RPCResult<u32, (), Self::Error>;

    /// Returns the epoch number for the current head.
    async fn get_epoch_number(&mut self) -> RPCResult<u32, (), Self::Error>;

    /// Tries to fetch a block given its hash. It has an option to include the transactions in the
    /// block, which defaults to false.
    async fn get_block_by_hash(
        &mut self,
        hash: Blake2bHash,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Tries to fetch a block given its number. It has an option to include the transactions in the
    /// block, which defaults to false. Note that this function will only fetch blocks that are part
    /// of the main chain.
    async fn get_block_by_number(
        &mut self,
        block_number: u32,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Returns the block at the head of the main chain. It has an option to include the
    /// transactions in the block, which defaults to false.
    async fn get_latest_block(
        &mut self,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Returns information about the proposer slot at the given block height and offset. The
    /// offset is optional, it will default to getting the offset for the existing block
    /// at the given height.
    /// We only have this information available for the last 2 batches at most.
    async fn get_slot_at(
        &mut self,
        block_number: u32,
        offset_opt: Option<u32>,
    ) -> RPCResult<Slot, BlockchainState, Self::Error>;

    /// Tries to fetch a transaction (including reward transactions) given its hash.
    async fn get_transaction_by_hash(
        &mut self,
        hash: Blake2bHash,
    ) -> RPCResult<ExecutedTransaction, (), Self::Error>;

    /// Returns all the transactions (including reward transactions) for the given block number. Note
    /// that this only considers blocks in the main chain.
    async fn get_transactions_by_block_number(
        &mut self,
        block_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    /// Returns all the inherents (including reward inherents) for the given block number. Note
    /// that this only considers blocks in the main chain.
    async fn get_inherents_by_block_number(
        &mut self,
        block_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error>;

    /// Returns all the transactions (including reward transactions) for the given batch number. Note
    /// that this only considers blocks in the main chain.
    async fn get_transactions_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    /// Returns all the inherents (including reward inherents) for the given batch number. Note
    /// that this only considers blocks in the main chain.  
    async fn get_inherents_by_batch_number(
        &mut self,
        batch_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error>;

    /// Returns the hashes for the latest transactions for a given address. All the transactions
    /// where the given address is listed as a recipient or as a sender are considered. Reward
    /// transactions are also returned. It has an option to specify the maximum number of hashes to
    /// fetch, it defaults to 500.
    async fn get_transaction_hashes_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> RPCResult<Vec<Blake2bHash>, (), Self::Error>;

    /// Returns the latest transactions for a given address. All the transactions
    /// where the given address is listed as a recipient or as a sender are considered. Reward
    /// transactions are also returned. It has an option to specify the maximum number of transactions
    /// to fetch, it defaults to 500.
    async fn get_transactions_by_address(
        &mut self,
        address: Address,
        max: Option<u16>,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    /// Tries to fetch the account at the given address.
    async fn get_account_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Account, BlockchainState, Self::Error>;

    /// Fetches all accounts in the accounts tree.
    /// IMPORTANT: This operation iterates over all accounts in the accounts tree
    /// and thus is extremely computationally expensive.
    async fn get_accounts(&mut self) -> RPCResult<Vec<Account>, BlockchainState, Self::Error>;

    /// Returns a collection of the currently active validator's addresses and balances.
    async fn get_active_validators(
        &mut self,
    ) -> RPCResult<Vec<Validator>, BlockchainState, Self::Error>;

    /// Returns information about the currently penalized slots. This includes slots that lost rewards
    /// and that were disabled.
    async fn get_current_penalized_slots(
        &mut self,
    ) -> RPCResult<PenalizedSlots, BlockchainState, Self::Error>;

    /// Returns information about the penalized slots of the previous batch. This includes slots that
    /// lost rewards and that were disabled.
    async fn get_previous_penalized_slots(
        &mut self,
    ) -> RPCResult<PenalizedSlots, BlockchainState, Self::Error>;

    /// Tries to fetch a validator information given its address.
    async fn get_validator_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Validator, BlockchainState, Self::Error>;

    /// Fetches all validators in the staking contract.
    /// IMPORTANT: This operation iterates over all validators in the staking contract
    /// and thus is extremely computationally expensive.
    async fn get_validators(&mut self) -> RPCResult<Vec<Validator>, BlockchainState, Self::Error>;

    /// Fetches all stakers for a given validator.
    /// IMPORTANT: This operation iterates over all stakers of the staking contract
    /// and thus is extremely computationally expensive.
    async fn get_stakers_by_validator_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Vec<Staker>, BlockchainState, Self::Error>;

    /// Tries to fetch a staker information given its address.
    async fn get_staker_by_address(
        &mut self,
        address: Address,
    ) -> RPCResult<Staker, BlockchainState, Self::Error>;

    /// Subscribes to new block events (retrieves the full block).
    #[stream]
    async fn subscribe_for_head_block(
        &mut self,
        include_body: Option<bool>,
    ) -> Result<BoxStream<'static, RPCData<Block, ()>>, Self::Error>;

    /// Subscribes to new block events (only retrieves the block hash).
    #[stream]
    async fn subscribe_for_head_block_hash(
        &mut self,
    ) -> Result<BoxStream<'static, RPCData<Blake2bHash, ()>>, Self::Error>;

    /// Subscribes to pre epoch validators events.
    #[stream]
    async fn subscribe_for_validator_election_by_address(
        &mut self,
        address: Address,
    ) -> Result<BoxStream<'static, RPCData<Validator, BlockchainState>>, Self::Error>;

    /// Subscribes to log events related to a given list of addresses and of any of the log types provided.
    /// If addresses is empty it does not filter by address. If log_types is empty it won't filter by log types.
    /// Thus the behavior is to assume all addresses or log_types are to be provided if the corresponding vec is empty.
    #[stream]
    async fn subscribe_for_logs_by_addresses_and_types(
        &mut self,
        addresses: Vec<Address>,
        log_types: Vec<LogType>,
    ) -> Result<BoxStream<'static, RPCData<BlockLog, BlockchainState>>, Self::Error>;
}
