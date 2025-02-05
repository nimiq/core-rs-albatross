use async_trait::async_trait;

use crate::types::RPCResult;

#[nimiq_jsonrpc_derive::proxy(name = "NetworkProxy", rename_all = "camelCase")]
#[async_trait]
pub trait NetworkInterface {
    type Error;

    /// Returns the peer ID of the local peer.
    ///
    /// **Parameters**: None. This method does not require any input.
    /// 
    /// **Returns**:
    /// - The local peer ID as a `String`.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": "string",
    ///   "id": 1
    /// }
    /// ```
    async fn get_peer_id(&mut self) -> RPCResult<String, (), Self::Error>;

    /// Returns the number of connected peers.
    ///
    /// **Parameters**:
    /// - `-c`, `--count` (optional): Displays only the number of peers connected.
    ///
    /// **Returns**:
    /// - The number of peers as a `usize`.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": number,
    /// }
    /// ```
    async fn get_peer_count(&mut self) -> RPCResult<usize, (), Self::Error>;

    /// Returns a list of peer IDs for all connected peers.
    ///
    /// **Parameters**: None. This method does not require any input.
    ///
    /// **Returns**:  
    /// A vector (`Vec<String>`) containing the peer IDs of all connected peers.
    ///
    /// **Response Example**:  
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": [
    ///     "string",
    ///     "string",
    ///     "string",
    ///     "string"
    ///   ],
    ///   "id": 1
    /// }
    /// ```
    async fn get_peer_list(&mut self) -> RPCResult<Vec<String>, (), Self::Error>;
}
