use async_trait::async_trait;
use nimiq_blockchain::interface::HistoryIndexInterface;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::{BlockchainProxy, BlockchainReadProxy};
use nimiq_hash::Blake2bHash;
use nimiq_rpc_interface::{
    eth_interface::history::HistoryInterface,
    types::{Block, ExecutedTransaction, RPCResult},
};

use crate::error::Error;

pub struct HistoryDispatcher {
    pub blockchain: BlockchainProxy,
}

impl HistoryDispatcher {
    pub fn new(blockchain: BlockchainProxy) -> Self {
        Self { blockchain }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl HistoryInterface for HistoryDispatcher {
    type Error = Error;

    async fn eth_getBlockTransactionCountByHash(
        &self,
        block_hash: Blake2bHash,
    ) -> RPCResult<u32, (), Self::Error> {
        // TODO: Full transactions are not necessary, only the number of transactions
        let block = self
            .eth_getBlockByHash(block_hash.clone(), true)
            .await?
            .data;

        if let Some(txns) = block.transactions {
            Ok((txns.len() as u32).into())
        } else {
            Err(Error::BlockNotFoundByHash(block_hash))
        }
    }

    async fn eth_getBlockTransactionCountByNumber(
        &self,
        block_number: u32,
    ) -> RPCResult<u32, (), Self::Error> {
        // TODO: Full transactions are not necessary, only the number of transactions
        let block = self.eth_getBlockByNumber(block_number, true).await?.data;

        if let Some(txns) = block.transactions {
            Ok((txns.len() as u32).into())
        } else {
            Err(Error::BlockNotFound(block_number))
        }
    }

    async fn eth_getBlockByHash(
        &self,
        hash: Blake2bHash,
        include_body: bool,
    ) -> RPCResult<Block, (), Self::Error> {
        let blockchain = &self.blockchain.read();

        // TODO: include_body (in eth this parameter means if full txns should be returned or just hashes)

        blockchain
            .get_block(&hash, include_body)
            .and_then(|block| Block::from_block(blockchain, block, include_body))
            .map_err(|_| Error::BlockNotFoundByHash(hash.clone()))
            .map(|block| block.into())
    }

    async fn eth_getBlockByNumber(
        &self,
        block_number: u32,
        include_body: bool,
    ) -> RPCResult<Block, (), Self::Error> {
        let blockchain = self.blockchain.read();

        // TODO: include_body (in eth this parameter means if full txns should be returned or just hashes)

        let block = blockchain
            .get_block_at(block_number, include_body)
            .map_err(|_| Error::BlockNotFound(block_number))?;

        Ok(Block::from_block(&blockchain, block, include_body)
            .map_err(|_| Error::BlockNotFound(block_number))?
            .into())
    }

    async fn eth_getTransactionByHash(
        &self,
        hash: Blake2bHash,
    ) -> RPCResult<ExecutedTransaction, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
            // Get all the historic transactions that correspond to this hash.
            let hist_tx = blockchain
                .history_store
                .history_index()
                .ok_or(Error::RequiresHistoryIndex)?
                .get_hist_tx_by_hash(&hash, None)
                .ok_or_else(|| Error::TransactionNotFound(hash.clone()))?;

            // Convert the historic transaction into a regular transaction. This will also convert
            // reward inherents.
            Ok(ExecutedTransaction::try_from_historic_transaction(
                hist_tx,
                Some(blockchain.block_number()),
            )
            .ok_or_else(|| Error::TransactionNotFound(hash.clone()))?
            .into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }
}
