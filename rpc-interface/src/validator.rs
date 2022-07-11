use async_trait::async_trait;

use nimiq_keys::Address;

#[nimiq_jsonrpc_derive::proxy(name = "ValidatorProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ValidatorInterface {
    type Error;

    async fn get_address(&mut self) -> Result<Address, Self::Error>;

    async fn get_signing_key(&mut self) -> Result<String, Self::Error>;

    async fn get_voting_key(&mut self) -> Result<String, Self::Error>;

    async fn set_automatic_activation(
        &mut self,
        automatic_activation: bool,
    ) -> Result<bool, Self::Error>;
}
