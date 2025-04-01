use async_trait::async_trait;
use nimiq_account::Account;
use nimiq_blockchain_proxy::{BlockchainProxy, BlockchainReadProxy};
use nimiq_keys::Address;
use nimiq_rpc_interface::types::RPCResult;

use nimiq_rpc_interface::eth_interface::state::StateInterface;

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
        &mut self,
        address: Address,
        tag: String,
    ) -> RPCResult<u32, (), Self::Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let account = blockchain
                .get_account_if_complete(&address)
                .ok_or(Error::NoConsensus)?;

            match account {
                Account::Basic(basic) => {
                    let balance = basic.balance;
                }
                Account::Vesting(_) => todo!(),
                Account::HTLC(_) => todo!(),
                Account::Staking(_) => todo!(),
            }

            // TODO: Need to convert coin to integer

            Ok(0.into())
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    async fn eth_getTransactionCount(
        &mut self,
        address: Address,
        tag: String,
    ) -> RPCResult<u32, (), Self::Error> {
        todo!()
    }
}
