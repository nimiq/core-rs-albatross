use async_trait::async_trait;
use nimiq_keys::Address;

use crate::types::RPCResult;

#[nimiq_jsonrpc_derive::proxy(name = "ValidatorProxy", rename_all = "camelCase")]
#[async_trait]
pub trait ValidatorInterface {
    type Error;

    async fn get_address(&mut self) -> RPCResult<Address, (), Self::Error>;

    async fn get_signing_key(&mut self) -> RPCResult<String, (), Self::Error>;

    async fn get_voting_key(&mut self) -> RPCResult<String, (), Self::Error>;

    async fn set_automatic_reactivation(
        &mut self,
        automatic_reactivate: bool,
    ) -> RPCResult<(), (), Self::Error>;

    async fn is_validator_elected(&mut self) -> RPCResult<bool, (), Self::Error>;

    async fn is_validator_synced(&mut self) -> RPCResult<bool, (), Self::Error>;
}
