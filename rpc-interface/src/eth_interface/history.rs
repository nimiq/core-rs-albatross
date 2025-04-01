use async_trait::async_trait;

use nimiq_hash::Blake2bHash;

use crate::types::{Block, ExecutedTransaction, RPCResult};

#[nimiq_jsonrpc_derive::proxy(name = "BlockchainProxy", rename_all = "camelCase")]
#[async_trait]
pub trait HistoryInterface {
    type Error;

    /// Returns the number of transactions sent from an address.
    async fn eth_getBlockTransactionCountByHash(
        &mut self,
        block_hash: Blake2bHash,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns the number of transactions in a block matching the given block number.
    async fn eth_getBlockTransactionCountByNumber(
        &mut self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns information about a block by hash.
    async fn eth_getBlockByHash(
        &mut self,
        hash: Blake2bHash,
        include_full_txns: bool,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Returns information about a block by block number.
    async fn eth_getBlockByNumber(
        &mut self,
        block_number: u32,
        include_full_txns: bool,
    ) -> RPCResult<Block, (), Self::Error>;

    /// Tries to fetch a transaction (including reward transactions) given its hash.
    async fn eth_getTransactionByHash(
        &mut self,
        hash: Blake2bHash,
    ) -> RPCResult<ExecutedTransaction, (), Self::Error>;
}
