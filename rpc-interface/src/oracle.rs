use async_trait::async_trait;
use nimiq_keys::Address;
use nimiq_transaction::account::htlc_contract::AnyHash;

use crate::types::RPCResult;

#[nimiq_jsonrpc_derive::proxy(name = "OracleProxy", rename_all = "camelCase")]
#[async_trait]
pub trait OracleInterface {
    type Error;

    /// Returns the current owner address of the oracle contract.
    async fn get_owner(&self, contract_address: Address) -> RPCResult<Address, (), Self::Error>;

    /// Returns the 0-based index of the last written hash.
    /// Returns an error if the oracle contract has no data yet.
    /// Valid indices for `get_entry` are in `[earliest_index, latest_index]` (inclusive).
    async fn get_latest_index(&self, contract_address: Address) -> RPCResult<u64, (), Self::Error>;

    /// Returns the 0-based earliest index still retained in the ring buffer.
    /// Returns an error if the oracle contract has no data yet.
    async fn get_earliest_index(
        &self,
        contract_address: Address,
    ) -> RPCResult<u64, (), Self::Error>;

    /// Returns the size of the sliding window (ring buffer capacity).
    async fn get_window_size(&self, contract_address: Address) -> RPCResult<u16, (), Self::Error>;

    /// Returns the latest hash-chain head.
    /// Returns an error if the oracle contract has no data yet.
    async fn get_latest_data(
        &self,
        contract_address: Address,
    ) -> RPCResult<AnyHash, (), Self::Error>;

    /// Returns the data for a given 0-based index.
    /// Index must be in `[earliest_index, latest_index]` (inclusive).
    async fn get_entry(
        &self,
        contract_address: Address,
        index: u64,
    ) -> RPCResult<AnyHash, (), Self::Error>;
}
