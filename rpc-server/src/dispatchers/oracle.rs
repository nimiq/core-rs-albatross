use async_trait::async_trait;
use nimiq_account::Account;
use nimiq_blockchain_proxy::{BlockchainProxy, BlockchainReadProxy};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_rpc_interface::{
    oracle::{OracleEntry, OracleInterface},
    types::RPCResult,
};
use nimiq_transaction::account::htlc_contract::AnyHash;

use crate::error::Error;

pub struct OracleDispatcher {
    pub blockchain: BlockchainProxy,
}

impl OracleDispatcher {
    pub fn new(blockchain: BlockchainProxy) -> Self {
        Self { blockchain }
    }

    /// Helper to get OracleContract from blockchain
    fn get_oracle_contract(
        &self,
        contract_address: Address,
    ) -> Result<nimiq_account::OracleContract, Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let account = blockchain
                .get_account_if_complete(&contract_address)
                .ok_or(Error::NoConsensus)?;

            match account {
                Account::Oracle(oracle) => Ok(oracle),
                _ => Err(Error::InvalidAddress(format!(
                    "Address {} is not an Oracle contract",
                    contract_address
                ))),
            }
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }

    /// Helper to convert AnyHash to Blake2bHash
    /// Returns an error if the hash is not Blake2b
    fn any_hash_to_blake2b(hash: &AnyHash) -> Result<Blake2bHash, Error> {
        match hash {
            AnyHash::Blake2b(h) => Ok(Blake2bHash::from(&h.0[..])),
            _ => Err(Error::InvalidData(format!(
                "Oracle contract uses non-Blake2b hash type, which is not supported for this RPC method"
            ))),
        }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl OracleInterface for OracleDispatcher {
    type Error = Error;

    async fn get_latest_index(&self, contract_address: Address) -> RPCResult<u64, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;
        Ok(oracle.latest_index.into())
    }

    async fn get_earliest_index(
        &self,
        contract_address: Address,
    ) -> RPCResult<u64, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;
        Ok(oracle.earliest_index().into())
    }

    async fn get_window_size(&self, contract_address: Address) -> RPCResult<u16, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;
        Ok(oracle.hash_count.into())
    }

    async fn get_latest_data(
        &self,
        contract_address: Address,
    ) -> RPCResult<Blake2bHash, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;

        if oracle.latest_index == 0 {
            return Err(Error::InvalidData(
                "Oracle contract has no data (latest_index is 0)".to_string(),
            ));
        }

        // Get the latest hash (at index latest_index - 1)
        let latest_index = oracle.latest_index - 1;
        let latest_hash = oracle.get_hash_at_index(latest_index).ok_or_else(|| {
            Error::InvalidData(format!("Failed to get hash at index {}", latest_index))
        })?;

        Self::any_hash_to_blake2b(latest_hash).map(|hash| hash.into())
    }

    async fn get_entry(
        &self,
        contract_address: Address,
        index: u64,
    ) -> RPCResult<OracleEntry, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;

        // Check if index is within the retained window
        let earliest = oracle.earliest_index();
        if index < earliest || index >= oracle.latest_index {
            return Err(Error::InvalidData(format!(
                "Index {} is outside the retained window [{}, {})",
                index, earliest, oracle.latest_index
            )));
        }

        // Get the hash at the given index
        let hash = oracle
            .get_hash_at_index(index)
            .ok_or_else(|| Error::InvalidData(format!("Failed to get hash at index {}", index)))?;

        let data = Self::any_hash_to_blake2b(hash)?;

        Ok(OracleEntry { data }.into())
    }
}
