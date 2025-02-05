use async_trait::async_trait;
use nimiq_hash::Blake2bHash;
use nimiq_transaction::Transaction;

use crate::types::{HashOrTx, MempoolInfo, RPCResult};

#[nimiq_jsonrpc_derive::proxy(name = "MempoolProxy", rename_all = "camelCase")]
#[async_trait]
pub trait MempoolInterface {
    type Error;

    /// Pushes a raw transaction with default priority into the mempool and broadcasts it to the network.
    ///
    /// **Parameters**:
    /// - `raw_tx` (String): The raw transaction, serialized and encoded in hex format, to be pushed to the mempool.
    /// 
    /// **Returns**:
    /// A `Blake2bHash` representing the hash of the transaction if it was successfully added to the mempool and broadcasted.
    ///
    /// **Response Example**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": "string",
    /// }
    /// ```
    async fn push_transaction(&mut self, raw_tx: String)
        -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Pushes a raw transaction with high priority into the mempool and broadcasts it to the network.
    /// 
    /// **Parameters**:
    /// - `raw_tx` (String): The raw transaction, serialized and encoded in hex format, to be pushed to the mempool.    
    ///     - This method corresponds to the `push_transaction` method with the additional `--high-priority` (`-p`) option.
    /// 
    /// **Returns**:
    /// A `Blake2bHash` representing the hash of the transaction if it was successfully added to the mempool and broadcasted.
    /// 
    /// **Response Example**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": "string",
    /// }
    /// ```
    async fn push_high_priority_transaction(
        &mut self,
        raw_tx: String,
    ) -> RPCResult<Blake2bHash, (), Self::Error>;

    /// Obtains the list of transactions currently in the mempool.
    /// 
    /// Some fields in the returned transactions, such as `block_number`, `timestamp`, 
    /// and `confirmations`, may be `null` if the transactions are pending and have 
    /// not yet been confirmed on the blockchain.
    ///
    /// **Parameters**: 
    ///   - `include_transactions` (`bool`): This parameter is set to `true` when the `include-transactions` or `-t` option is used, causing the response to include full transaction details. Otherwise, only transaction hashes are returned.
    ///
    /// **Returns**: 
    /// A vector (`Vec<HashOrTx>`) containing:
    /// - Transaction hashes (`Blake2bHash`) when `include_transactions` is `false`.
    /// - Full transaction details (`Transaction`) when `include_transactions` is `true`.
    ///
    /// **Response Example**:
    /// With `include_transactions = false`:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": ["string", "string"],
    ///   "id": 1
    /// }
    /// ```
    ///
    /// With `include_transactions = true`:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     {
    ///       "hash": "string",
    ///       "block_number": number,
    ///       "timestamp": number,
    ///       "confirmations": number,
    ///       "size": number,
    ///       "related_addresses": [
    ///         "string",
    ///         "string"
    ///       ],
    ///       "from": "string",
    ///       "from_type": number,
    ///       "to": "string",
    ///       "to_type": number,
    ///       "value": number,
    ///       "fee": number,
    ///       "sender_data": "string",
    ///       "recipient_data": "string",
    ///       "flags": string,
    ///       "validity_start_height": number,
    ///       "proof": "string",
    ///       "network_id": number
    ///     }
    ///   ],
    /// }
    /// ```
    
    async fn mempool_content(
        &mut self,
        include_transactions: bool,
    ) -> RPCResult<Vec<HashOrTx>, (), Self::Error>;

    /// Obtains the mempool content grouped into fee-per-byte buckets.
    ///
    /// **Parameters**: None. This method does not take any input.
    ///
    /// **Returns**: 
    /// A `MempoolInfo` object with:
    /// - `total` (`u32`): Total transactions in the mempool.
    /// - `buckets` (`Vec<u64>`): Number of transactions in predefined fee-per-byte ranges.
    ///
    /// **Response Example**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "total": number,
    ///     "buckets": [array]
    ///   },
    /// }
    /// ```
    ///
    /// **Notes**:
    /// - Fee-per-byte ranges are predefined (e.g., 1, 2, 5, 10...).
    /// - Empty buckets are represented as `None`.
    async fn mempool(&mut self) -> RPCResult<MempoolInfo, (), Self::Error>;

    /// Obtains the minimum fee per byte as per mempool configuration. Transactions with fees below this value will be rejected by the mempool.
    /// 
    /// **Parameters**: None. This method does not require any input.
    /// 
    /// **Returns**: A `f64` value representing the minimum fee per byte, expressed in Luna.
    /// 
    /// **Response Example**:
    /// ```json
    ///   {
    ///     "jsonrpc": "2.0",
    ///     "result": number
    ///   }
    /// ```
    async fn get_min_fee_per_byte(&mut self) -> RPCResult<f64, (), Self::Error>;

    /// Retrieves the details of a specific transaction from the mempool, using its hash.
    ///
    /// **Parameters**:  
    /// - `hash` (*Blake2bHash*): The unique hash identifying the transaction.
    ///
    /// **Returns**:  
    /// - The transaction details (`Transaction`) if the transaction is found in the mempool.  
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///       "hash": "string",
    ///       "block_number": number,
    ///       "timestamp": number,
    ///       "confirmations": number,
    ///       "size": number,
    ///       "related_addresses": [
    ///         "string",
    ///         "string"
    ///       ],
    ///       "from": "string",
    ///       "from_type": number,
    ///       "to": "string",
    ///       "to_type": number,
    ///       "value": number,
    ///       "fee": number,
    ///       "sender_data": "string",
    ///       "recipient_data": "string",
    ///       "flags": number,
    ///       "validity_start_height": number,
    ///       "proof": "string",
    ///       "network_id": number
    ///   },
    /// }
    /// ```
    async fn get_transaction_from_mempool(
        &mut self,
        hash: Blake2bHash,
    ) -> RPCResult<Transaction, (), Self::Error>;
}
