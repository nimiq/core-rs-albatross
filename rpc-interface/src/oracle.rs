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
    /// Valid indices for `get_entry` are in `[0, latest_index]` (inclusive).
    async fn get_latest_index(&self, contract_address: Address) -> RPCResult<u64, (), Self::Error>;

    /// Returns the 0-based earliest index whose slot still holds the entry written at it.
    /// Returns an error if the oracle contract has no data yet.
    async fn get_earliest_index(
        &self,
        contract_address: Address,
    ) -> RPCResult<u64, (), Self::Error>;

    /// Returns the size of the sliding window (ring buffer capacity).
    async fn get_window_size(&self, contract_address: Address) -> RPCResult<u16, (), Self::Error>;

    /// Returns the current value of the slot written at the latest index.
    /// Returns an error if the oracle contract has no data yet.
    async fn get_latest_data(
        &self,
        contract_address: Address,
    ) -> RPCResult<AnyHash, (), Self::Error>;

    /// Returns the current value of the ring-buffer slot that a 0-based index was written to,
    /// `index mod window_size`, which a burn proof for that index must end in. For an index in
    /// `[earliest_index, latest_index]` this is the entry written at it; an older index still names
    /// its slot, whose entry has since been extended by one Merkle node per rotation.
    /// Index must be in `[0, latest_index]` (inclusive).
    async fn get_entry(
        &self,
        contract_address: Address,
        index: u64,
    ) -> RPCResult<AnyHash, (), Self::Error>;
}
