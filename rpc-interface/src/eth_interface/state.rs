use async_trait::async_trait;

use nimiq_keys::Address;

use crate::types::RPCResult;

#[nimiq_jsonrpc_derive::proxy(name = "BlockchainProxy", rename_all = "camelCase")]
#[async_trait]
pub trait StateInterface {
    type Error;

    /// Returns the balance of the account of given address.
    async fn eth_getBalance(
        &mut self,
        address: Address,
        tag: String,
    ) -> RPCResult<u32, (), Self::Error>;

    /// Returns the number of transactions sent from an address.
    async fn eth_getTransactionCount(
        &mut self,
        address: Address,
        tag: String,
    ) -> RPCResult<u32, (), Self::Error>;
}
