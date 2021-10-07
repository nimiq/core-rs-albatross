use async_trait::async_trait;
use beserial::Serialize;

use nimiq_keys::Address;
use nimiq_rpc_interface::validator::ValidatorInterface;
use nimiq_validator::validator::ValidatorProxy;

use crate::error::Error;

pub struct ValidatorDispatcher {
    validator: ValidatorProxy,
}

impl ValidatorDispatcher {
    pub fn new(validator: ValidatorProxy) -> Self {
        ValidatorDispatcher { validator }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl ValidatorInterface for ValidatorDispatcher {
    type Error = Error;

    async fn get_address(&mut self) -> Result<Address, Self::Error> {
        Ok(self.validator.validator_address.read().clone())
    }

    async fn get_warm_key(&mut self) -> Result<String, Self::Error> {
        Ok(hex::encode(
            self.validator.warm_key.read().private.serialize_to_vec(),
        ))
    }

    async fn get_hot_key(&mut self) -> Result<String, Self::Error> {
        Ok(hex::encode(
            self.validator
                .signing_key
                .read()
                .secret_key
                .serialize_to_vec(),
        ))
    }
}
