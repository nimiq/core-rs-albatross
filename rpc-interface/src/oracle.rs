use async_trait::async_trait;
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;

use crate::types::RPCResult;

/// Oracle entry containing height and data
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OracleEntry {
    /// The hash data for this entry
    pub data: Blake2bHash,
}

#[nimiq_jsonrpc_derive::proxy(name = "OracleProxy", rename_all = "camelCase")]
#[async_trait]
pub trait OracleInterface {
    type Error;

    /// Returns the latest index (number of updates performed so far).
    async fn get_latest_index(&self, contract_address: Address) -> RPCResult<u64, (), Self::Error>;

    /// Returns the earliest index still retained in the ring buffer.
    async fn get_earliest_index(
        &self,
        contract_address: Address,
    ) -> RPCResult<u64, (), Self::Error>;

    /// Returns the size of the sliding window (ring buffer capacity).
    async fn get_window_size(&self, contract_address: Address) -> RPCResult<u16, (), Self::Error>;

    /// Returns the latest hash-chain head.
    async fn get_latest_data(
        &self,
        contract_address: Address,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Returns the data for a given index.
    /// Returns an error if index is outside the currently retained window.
    async fn get_entry(
        &self,
        contract_address: Address,
        index: u64,
    ) -> RPCResult<OracleEntry, (), Self::Error>;
}
