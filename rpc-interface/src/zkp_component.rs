use async_trait::async_trait;

use crate::types::{RPCResult, ZKPState};

#[nimiq_jsonrpc_derive::proxy(name = "ZKPComponentProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ZKPComponentInterface {
    type Error;

    /// Retrieves the current ZKP state, including the latest proof and its associated block details.
    /// 
    /// **Parameters**: None. This method does not require any input.
    /// 
    /// **Returns**: 
    /// A `ZKPState` object containing:
    /// - `latest_block`: Details of the block associated with the latest proof, including its hash, size, epoch, batch, seed, state hash, and other relevant fields.
    /// - `latest_proof`: The Zero-Knowledge Proof for the latest block, represented as cryptographic fields.
    /// 
    /// **Example Response**:
    /// ```json
    /// {
    ///   "data": {
    ///     "latest_block": {
    ///       "hash": "string",
    ///       "size": number,
    ///       "batch": number,
    ///       "epoch": number,
    ///       "number": number,
    ///       "timestamp": number,
    ///       "parentHash": "string",
    ///       "seed": "string",
    ///       "stateHash": "string",
    ///       "bodyHash": "string",
    ///       "historyHash": "string",
    ///       "additionalFields": {
    ///         "isElectionBlock": false,
    ///         "parentElectionHash": "string",
    ///         "interlink": "string"
    ///       }
    ///     },
    ///     "latestProof": {
    ///       "a": "string",
    ///       "b": "string",
    ///       "c": "string"
    ///     }
    ///   },
    ///   "metadata": null
    /// }
    /// ```
    async fn get_zkp_state(&mut self) -> RPCResult<ZKPState, (), Self::Error>;
}
