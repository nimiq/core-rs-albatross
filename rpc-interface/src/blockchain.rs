use async_trait::async_trait;
use futures::stream::BoxStream;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_primitives::networks::NetworkId;

use crate::types::{
    Account, Block, BlockLog, BlockchainState, ExecutedTransaction, Inherent, LogType,
    MerklePathData, PenalizedSlots, RPCData, RPCResult, Slot, Staker, Validator,
};

#[nimiq_jsonrpc_derive::proxy(name = "BlockchainProxy", rename_all = "camelCase")]
#[async_trait]
pub trait BlockchainInterface {
    type Error;

    /// Returns the network ID.
    async fn get_network_id(&self) -> RPCResult<NetworkId, (), Self::Error>;

    /// Returns the block number for the current head.
    async fn get_block_number(&self) -> RPCResult<u32, (), Self::Error>;

    /// Returns the batch number for the current head.
    async fn get_batch_number(&self) -> RPCResult<u32, (), Self::Error>;

    /// Returns the epoch number for the current head.
    async fn get_epoch_number(&self) -> RPCResult<u32, (), Self::Error>;

    /// Tries to fetch a block given its hash. It has an option to include the transactions in the
    /// block, which defaults to false.
    async fn get_block_by_hash(
        &self,
        hash: Blake2bHash,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Tries to fetch a block given its number. It has an option to include the transactions in the
    /// block, which defaults to false. Note that this function will only fetch blocks that are part
    /// of the main chain.
    async fn get_block_by_number(
        &self,
        block_number: u32,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Returns the block at the head of the main chain. It has an option to include the
    /// transactions in the block, which defaults to false.
    async fn get_latest_block(
        &self,
        include_body: Option<bool>,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Returns information about the proposer slot at the given block height and offset. The
    /// offset is optional, it will default to getting the offset for the existing block
    /// at the given height.
    /// We only have this information available for the last 2 batches at most.
    async fn get_slot_at(
        &self,
        block_number: u32,
        offset_opt: Option<u32>,
    ) -> RPCResult<Slot, BlockchainState, Self::Error>;

    /// Tries to fetch a transaction (including reward transactions) given its hash.
    async fn get_transaction_by_hash(
        &self,
        hash: Blake2bHash,
    ) -> RPCResult<ExecutedTransaction, (), Self::Error>;

    /// Returns the raw transaction (hex-encoded) given its hash.
    async fn get_raw_transaction_by_hash(
        &self,
        hash: Blake2bHash,
    ) -> RPCResult<String, (), Self::Error>;

    /// Returns all the transactions (including reward transactions) for the given block number. Note
    /// that this only considers blocks in the main chain.
    async fn get_transactions_by_block_number(
        &self,
        block_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    /// Returns all the inherents (including reward inherents) for the given block number. Note
    /// that this only considers blocks in the main chain.
    async fn get_inherents_by_block_number(
        &self,
        block_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error>;

    /// Returns all the transactions (including reward transactions) for the given batch number. Note
    /// that this only considers blocks in the main chain.
    async fn get_transactions_by_batch_number(
        &self,
        batch_number: u32,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    /// Returns all the inherents (including reward inherents) for the given batch number. Note
    /// that this only considers blocks in the main chain.
    async fn get_inherents_by_batch_number(
        &self,
        batch_number: u32,
    ) -> RPCResult<Vec<Inherent>, (), Self::Error>;

    /// Returns the hashes for the latest transactions for a given address. All the transactions
    /// where the given address is listed as a recipient or as a sender are considered. Reward
    /// transactions are also returned. It has an option to specify the maximum number of hashes to
    /// fetch, it defaults to 500. It has also an option to retrieve transactions before a given
    /// transaction hash (exclusive). If this hash is not found or does not belong to this address, it will return an empty list.
    /// The transaction hashes are returned in descending order, meaning the latest transaction is the first.
    async fn get_transaction_hashes_by_address(
        &self,
        address: Address,
        max: Option<u16>,
        start_at: Option<Blake2bHash>,
    ) -> RPCResult<Vec<Blake2bHash>, (), Self::Error>;

    /// Returns the latest transactions for a given address. All the transactions
    /// where the given address is listed as a recipient or as a sender are considered. Reward
    /// transactions are also returned. It has an option to specify the maximum number of transactions
    /// to fetch, it defaults to 500. It has also an option to retrieve transactions before a given
    /// transaction hash (exclusive). If this hash is not found or does not belong to this address, it will return an empty list.
    /// The transactions are returned in descending order, meaning the latest transaction is the first.
    async fn get_transactions_by_address(
        &self,
        address: Address,
        max: Option<u16>,
        start_at: Option<Blake2bHash>,
    ) -> RPCResult<Vec<ExecutedTransaction>, (), Self::Error>;

    /// Returns the transactions receipts (similar to get transactions by address)
    async fn get_transaction_references_by_address(
        &self,
        address: Address,
        max: Option<u16>,
        start_at: Option<Blake2bHash>,
    ) -> RPCResult<Vec<(Blake2bHash, u32)>, (), Self::Error>;

    /// Tries to fetch the account at the given address.
    async fn get_account_by_address(
        &self,
        address: Address,
    ) -> RPCResult<Account, BlockchainState, Self::Error>;

    /// Fetches all accounts in the accounts tree.
    /// IMPORTANT: This operation iterates over all accounts in the accounts tree
    /// and thus is extremely computationally expensive.
    async fn get_accounts(&self) -> RPCResult<Vec<Account>, BlockchainState, Self::Error>;

    /// Returns a collection of the currently active validator's addresses and balances.
    async fn get_active_validators(
        &self,
    ) -> RPCResult<Vec<Validator>, BlockchainState, Self::Error>;

    /// Returns information about the currently penalized slots. This includes slots that lost rewards
    /// and that were disabled.
    async fn get_current_penalized_slots(
        &self,
    ) -> RPCResult<PenalizedSlots, BlockchainState, Self::Error>;

    /// Returns information about the penalized slots of the previous batch. This includes slots that
    /// lost rewards and that were disabled.
    async fn get_previous_penalized_slots(
        &self,
    ) -> RPCResult<PenalizedSlots, BlockchainState, Self::Error>;

    /// Tries to fetch a validator information given its address.
    async fn get_validator_by_address(
        &self,
        address: Address,
    ) -> RPCResult<Validator, BlockchainState, Self::Error>;

    /// Fetches all validators in the staking contract.
    /// IMPORTANT: This operation iterates over all validators in the staking contract
    /// and thus is extremely computationally expensive.
    async fn get_validators(&self) -> RPCResult<Vec<Validator>, BlockchainState, Self::Error>;

    /// Fetches all stakers for a given validator.
    /// IMPORTANT: This operation iterates over all stakers of the staking contract
    /// and thus is extremely computationally expensive.
    async fn get_stakers_by_validator_address(
        &self,
        address: Address,
    ) -> RPCResult<Vec<Staker>, BlockchainState, Self::Error>;

    /// Tries to fetch a staker information given its address.
    async fn get_staker_by_address(
        &self,
        address: Address,
    ) -> RPCResult<Staker, BlockchainState, Self::Error>;

    /// Gets the nonce for a given address in a bridge contract.
    /// Returns the current nonce value, or 0 if no nonce has been set yet.
    async fn get_bridge_nonce(
        &self,
        bridge_address: Address,
        address: Address,
    ) -> RPCResult<u64, BlockchainState, Self::Error>;

    /// Subscribes to new block events (retrieves the full block).
    #[stream]
    async fn subscribe_for_head_block(
        &self,
        include_body: Option<bool>,
    ) -> Result<BoxStream<'static, RPCData<Block, ()>>, Self::Error>;

    /// Subscribes to new block events (only retrieves the block hash).
    #[stream]
    async fn subscribe_for_head_block_hash(
        &self,
    ) -> Result<BoxStream<'static, RPCData<Blake2bHash, ()>>, Self::Error>;

    /// Subscribes to pre epoch validators events.
    #[stream]
    async fn subscribe_for_validator_election_by_address(
        &self,
        address: Address,
    ) -> Result<BoxStream<'static, RPCData<Validator, BlockchainState>>, Self::Error>;

    /// Subscribes to log events related to a given list of addresses and of any of the log types provided.
    /// If addresses is empty it does not filter by address. If log_types is empty it won't filter by log types.
    /// Thus the behavior is to assume all addresses or log_types are to be provided if the corresponding vec is empty.
    #[stream]
    async fn subscribe_for_logs_by_addresses_and_types(
        &self,
        addresses: Vec<Address>,
        log_types: Vec<LogType>,
    ) -> Result<BoxStream<'static, RPCData<BlockLog, BlockchainState>>, Self::Error>;

    /// Gets the Keccak256 history root for a given epoch.
    async fn get_keccak256_history_root(
        &self,
        block_number: u32,
    ) -> RPCResult<String, (), Self::Error>;

    /// Gets a Keccak256-based Merkle proof for a specific transaction in an epoch.
    async fn get_keccak256_transaction_proof(
        &self,
        block_number: u32,
        transaction_hash: Blake2bHash,
    ) -> RPCResult<MerklePathData, (), Self::Error>;
}
