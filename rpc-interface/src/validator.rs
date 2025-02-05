use async_trait::async_trait;
use nimiq_keys::Address;

use crate::types::RPCResult;

#[nimiq_jsonrpc_derive::proxy(name = "ValidatorProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ValidatorInterface {
    type Error;

    /// Returns the validator address of our node.
    ///
    /// This method is only available for validator nodes, meaning the validator section must be uncommented in the client.toml configuration file.
    /// If not enabled, the method will return an error.
    ///
    /// **Returns**:
    /// - `Address`: The validator address of the node.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": "string",
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_address(&mut self) -> RPCResult<Address, (), Self::Error>;

    /// Returns the validator signing key of our node.
    ///
    /// This method is only available for validator nodes, meaning the validator section must be uncommented in the client.toml configuration file.
    /// If not enabled, the method will return an error.
    ///
    /// **Returns**:
    /// - `String`: The signing key of the validator node.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": "string",
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_signing_key(&mut self) -> RPCResult<String, (), Self::Error>;

    /// Returns the validator voting key of our node.
    ///
    /// This method is only available for validator nodes, meaning the validator section must be uncommented in the client.toml configuration file.
    /// If not enabled, the method will return an error.
    ///
    /// **Returns**:
    /// - `String`: The voting key of the validator node.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": "string",
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_voting_key(&mut self) -> RPCResult<String, (), Self::Error>;

    /// Returns all available voting keys.
    ///
    /// This method is only available for validator nodes, meaning the validator section must be uncommented in the client.toml configuration file.
    /// If not enabled, the method will return an error.
    ///
    /// **Returns**:  
    /// - A list of available voting keys (`Vec<String>`).
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": ["string", "string"],  
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn get_voting_keys(&mut self) -> RPCResult<Vec<String>, (), Self::Error>;

    /// Adds a new voting key that will be used when the key expected by the chain changes.
    ///
    /// This method is only available for validator nodes, meaning the validator section must be uncommented in the client.toml configuration file.
    /// If not enabled, the method will return an error.
    /// 
    /// **Parameters**:
    /// - `secret_key` (`String`): The secret key of the voting key to be added.
    ///
    /// **Returns**:  
    /// - An empty result indicating success.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": null,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn add_voting_key(&mut self, secret_key: String) -> RPCResult<(), (), Self::Error>;

    /// Updates the configuration setting to automatically reactivate the validator.
    ///
    /// This method is only available for validator nodes, meaning the validator section must be uncommented in the client.toml configuration file.
    /// If not enabled, the method will return an error.
    ///
    /// **Parameters**:
    /// - `automatic_reactivate` (`bool`): Whether the validator should automatically reactivate (`true`) or not (`false`).
    ///
    /// **Returns**:  
    /// - An empty result indicating success.
    ///
    /// **Example Response (Successful Update)**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": null,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn set_automatic_reactivation(
        &mut self,
        automatic_reactivate: bool,
    ) -> RPCResult<(), (), Self::Error>;

    /// Returns if our validator is currently elected.
    ///
    /// This method is only available for validator nodes, meaning the validator section must be uncommented in the client.toml configuration file.
    /// If not enabled, the method will return an error.
    ///
    /// **Returns**:  
    /// - `bool`: `true` if the validator is currently elected, `false` otherwise.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": true,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn is_validator_elected(&mut self) -> RPCResult<bool, (), Self::Error>;

    /// Returns if our validator is currently synced.
    /// 
    /// This method is only available for validator nodes, meaning the validator section must be uncommented in the client.toml configuration file.
    /// If not enabled, the method will return an error.
    /// 
    /// **Returns**:  
    /// - `bool`: `true` if the validator is fully synced, `false` otherwise.
    ///
    /// **Example Response**:
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "data": true,
    ///     "metadata": null
    ///   },
    /// }
    /// ```
    async fn is_validator_synced(&mut self) -> RPCResult<bool, (), Self::Error>;
}
