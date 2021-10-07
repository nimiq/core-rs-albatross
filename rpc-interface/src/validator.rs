use async_trait::async_trait;

use nimiq_keys::Address;

#[cfg_attr(
    feature = "proxy",
    nimiq_jsonrpc_derive::proxy(name = "ValidatorProxy", rename_all = "camelCase")
)]
#[async_trait]
pub trait ValidatorInterface {
    type Error;

    async fn get_address(&mut self) -> Result<Address, Self::Error>;

    async fn get_warm_key(&mut self) -> Result<String, Self::Error>;

    async fn get_hot_key(&mut self) -> Result<String, Self::Error>;
}
