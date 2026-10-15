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

/// The current value of the slot that `index` was written to. An index older than the window
/// still names its slot, so only an index that has not been written yet is an error.
fn entry(oracle: &OracleContract, index: u64) -> Result<AnyHash, Error> {
    let latest = latest_index(oracle)?;
    oracle.get_hash_at_index(index).cloned().ok_or_else(|| {
        Error::InvalidData(format!(
            "Index {} has not been written yet, the latest index is {}",
            index, latest
        ))
    })
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
        Ok(entry(&oracle, index)?.into())
    }
}

#[cfg(test)]
mod tests {
    use nimiq_account::OracleContract;
    use nimiq_keys::Address;
    use nimiq_primitives::coin::Coin;
    use nimiq_transaction::account::htlc_contract::{AnyHash, AnyHash32};

    use super::{earliest_index, entry, latest_index};
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
        assert!(matches!(
            entry(&oracle, 0),
            Err(Error::InvalidData(message)) if message == "Oracle contract has no data"
        ));
    }

    #[test]
    fn an_index_older_than_the_window_still_names_its_slot() {
        let slot = |tag: u8| AnyHash::Blake2b(AnyHash32([tag; 32]));
        // Four slots after indices 0..=5: slots 0 and 1 were written again at 4 and 5.
        let oracle = OracleContract {
            owner: Address([0u8; 20]),
            balance: Coin::ZERO,
            hash_count: 4,
            hashes: vec![slot(4), slot(5), slot(2), slot(3)],
            latest_index: Some(5),
        };
        assert_eq!(earliest_index(&oracle).ok(), Some(2));

        for index in 0..=5 {
            assert_eq!(
                entry(&oracle, index).ok(),
                Some(slot([4, 5, 2, 3][index as usize % 4])),
                "index {index}"
            );
        }
        assert!(matches!(
            entry(&oracle, 6),
            Err(Error::InvalidData(message)) if message.contains("has not been written yet")
        ));
    }
}
