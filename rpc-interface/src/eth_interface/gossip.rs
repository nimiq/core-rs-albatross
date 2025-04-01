use async_trait::async_trait;
use nimiq_hash::Blake2bHash;

use crate::types::RPCResult;

#[nimiq_jsonrpc_derive::proxy(name = "BlockchainProxy", rename_all = "camelCase")]
#[async_trait]
pub trait GossipInterface {
    type Error;

    /// Returns the number of most recent block.
    async fn eth_blockNumber(&mut self) -> RPCResult<u32, (), Self::Error>;

    /// Creates new message call transaction for signed transactions.
    async fn eth_sendRawTransaction(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;
}
