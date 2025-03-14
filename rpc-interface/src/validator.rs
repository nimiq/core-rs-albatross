use async_trait::async_trait;
use nimiq_keys::Address;

use crate::types::{RPCResult, ValidatorHealth};

#[nimiq_jsonrpc_derive::proxy(name = "ValidatorProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ValidatorInterface {
    type Error;

    /// Returns our validator address.
    async fn get_address(&mut self) -> RPCResult<Address, (), Self::Error>;

    /// Returns our validator signing key.
    async fn get_signing_key(&mut self) -> RPCResult<String, (), Self::Error>;

    /// Returns our current validator voting key.
    async fn get_voting_key(&mut self) -> RPCResult<String, (), Self::Error>;

    /// Returns all available voting keys.
    async fn get_voting_keys(&mut self) -> RPCResult<Vec<String>, (), Self::Error>;

    /// Returns the current validator health.
    async fn get_validator_health(&mut self) -> RPCResult<ValidatorHealth, (), Self::Error>;

    // Adds a voting key that will be used when the key expected by the chain changes
    async fn add_voting_key(&mut self, secret_key: String) -> RPCResult<(), (), Self::Error>;

    /// Updates the configuration setting to automatically reactivate our validator.
    async fn set_automatic_reactivation(
        &mut self,
        automatic_reactivate: bool,
    ) -> RPCResult<(), (), Self::Error>;

    /// Returns if our validator is currently elected.
    async fn is_validator_elected(&mut self) -> RPCResult<bool, (), Self::Error>;

    /// Returns if our validator is currently synced.
    async fn is_validator_synced(&mut self) -> RPCResult<bool, (), Self::Error>;
}
