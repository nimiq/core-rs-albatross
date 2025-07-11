use async_trait::async_trait;
use nimiq_account::Account;
use nimiq_blockchain::interface::HistoryIndexInterface;
use nimiq_blockchain_proxy::{BlockchainProxy, BlockchainReadProxy};
use nimiq_keys::Address;
use nimiq_primitives::coin::Coin;
use nimiq_rpc_interface::{eth_interface::state::StateInterface, types::RPCResult};

use crate::error::Error;

pub struct StateDispatcher {
    pub blockchain: BlockchainProxy,
}

impl StateDispatcher {
    pub fn new(blockchain: BlockchainProxy) -> Self {
        Self { blockchain }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl StateInterface for StateDispatcher {
    type Error = Error;

    async fn eth_getBalance(
        &self,
        address: Address,
        _tag: String,
    ) -> RPCResult<u32, (), Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let account = blockchain
                .get_account_if_complete(&address)
                .ok_or(Error::NoConsensus)?;

            let balance = match account {
                Account::Basic(basic) => basic.balance,
                _ => Coin::from_u64_unchecked(0), // TODO: Contracts are not supported yet
            };

            // Convert Coin to u32, capping at u32::MAX if overflow occurs
            let balance_u32: u32 = u64::from(balance).try_into().unwrap_or(u32::MAX);

            Ok(balance_u32.into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    async fn eth_getTransactionCount(
        &self,
        address: Address,
        _tag: String,
    ) -> RPCResult<u32, (), Self::Error> {
        if let BlockchainReadProxy::Full(blockchain) = self.blockchain.read() {
            // Get all the historic transactions that correspond to this hash.
            let tx_hashes = blockchain
                .history_store
                .history_index()
                .ok_or(Error::RequiresHistoryIndex)?
                .get_tx_hashes_by_address(&address, 100, None, None);

            // Convert the historic transaction into a regular transaction. This will also convert
            // reward inherents.
            Ok((tx_hashes.len() as u32).into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }
}
