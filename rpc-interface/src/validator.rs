use async_trait::async_trait;
use nimiq_keys::Address;

use crate::types::RPCResult;

#[nimiq_jsonrpc_derive::proxy(name = "ValidatorProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ValidatorInterface {
    type Error;

    /// Returns our validator address.
    async fn get_address(&self) -> RPCResult<Address, (), Self::Error>;

    // Adds a voting key that will be used when the key expected by the chain changes
    async fn add_voting_key(&self, secret_key: String) -> RPCResult<(), (), Self::Error>;

    /// Updates the configuration setting to automatically reactivate our validator.
    async fn set_automatic_reactivation(
        &self,
        automatic_reactivate: bool,
    ) -> RPCResult<(), (), Self::Error>;

    /// Returns if our validator is currently elected.
    async fn is_validator_elected(&self) -> RPCResult<bool, (), Self::Error>;

    /// Returns if our validator is currently synced.
    async fn is_validator_synced(&self) -> RPCResult<bool, (), Self::Error>;
}
