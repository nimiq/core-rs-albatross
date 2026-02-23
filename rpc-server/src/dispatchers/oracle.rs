use async_trait::async_trait;
use nimiq_account::{Account, OracleContract};
use nimiq_blockchain_proxy::{BlockchainProxy, BlockchainReadProxy};
use nimiq_keys::Address;
use nimiq_rpc_interface::{oracle::OracleInterface, types::RPCResult};
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
    fn get_oracle_contract(&self, contract_address: Address) -> Result<OracleContract, Error> {
        let blockchain_proxy = self.blockchain.read();
        if let BlockchainReadProxy::Full(ref blockchain) = blockchain_proxy {
            let account = blockchain
                .get_account_if_complete(&contract_address)
                .ok_or(Error::NoConsensus)?;

            match account {
                Account::Oracle(oracle) => Ok(oracle),
                _ => Err(Error::InvalidAddress(contract_address)),
            }
        } else {
            Err(Error::NotSupportedForLightBlockchain)
        }
    }
}

fn empty_oracle_data_error() -> Error {
    Error::InvalidData("Oracle contract has no data".to_string())
}

fn latest_index(oracle: &OracleContract) -> Result<u64, Error> {
    oracle.latest_index.ok_or_else(empty_oracle_data_error)
}

fn earliest_index(oracle: &OracleContract) -> Result<u64, Error> {
    oracle.earliest_index().ok_or_else(empty_oracle_data_error)
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl OracleInterface for OracleDispatcher {
    type Error = Error;

    async fn get_owner(&self, contract_address: Address) -> RPCResult<Address, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;
        Ok(oracle.owner.into())
    }

    async fn get_latest_index(&self, contract_address: Address) -> RPCResult<u64, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;
        Ok(latest_index(&oracle)?.into())
    }

    async fn get_earliest_index(
        &self,
        contract_address: Address,
    ) -> RPCResult<u64, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;
        Ok(earliest_index(&oracle)?.into())
    }

    async fn get_window_size(&self, contract_address: Address) -> RPCResult<u16, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;
        Ok(oracle.hash_count.into())
    }

    async fn get_latest_data(
        &self,
        contract_address: Address,
    ) -> RPCResult<AnyHash, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;

        let latest_index = latest_index(&oracle)?;

        // Get the latest hash at latest_index
        let latest_hash = oracle.get_hash_at_index(latest_index).ok_or_else(|| {
            Error::InvalidData(format!("Failed to get hash at index {}", latest_index))
        })?;

        Ok(latest_hash.clone().into())
    }

    async fn get_entry(
        &self,
        contract_address: Address,
        index: u64,
    ) -> RPCResult<AnyHash, (), Self::Error> {
        let oracle = self.get_oracle_contract(contract_address)?;

        // Check if index is within the retained window
        let earliest = earliest_index(&oracle)?;
        let latest = latest_index(&oracle)?;
        if index < earliest || index > latest {
            return Err(Error::InvalidData(format!(
                "Index {} is outside the retained window [{}, {}]",
                index, earliest, latest
            )));
        }

        // Get the hash at the given index
        let hash = oracle
            .get_hash_at_index(index)
            .ok_or_else(|| Error::InvalidData(format!("Failed to get hash at index {}", index)))?;

        Ok(hash.clone().into())
    }
}

#[cfg(test)]
mod tests {
    use nimiq_account::OracleContract;
    use nimiq_keys::Address;
    use nimiq_primitives::coin::Coin;

    use super::{earliest_index, latest_index};
    use crate::error::Error;

    #[test]
    fn empty_oracle_indices_return_error() {
        let oracle = OracleContract {
            owner: Address([0u8; 20]),
            balance: Coin::ZERO,
            hash_count: 4,
            hashes: Vec::new(),
            latest_index: None,
        };

        assert!(matches!(
            latest_index(&oracle),
            Err(Error::InvalidData(message)) if message == "Oracle contract has no data"
        ));
        assert!(matches!(
            earliest_index(&oracle),
            Err(Error::InvalidData(message)) if message == "Oracle contract has no data"
        ));
    }
}
