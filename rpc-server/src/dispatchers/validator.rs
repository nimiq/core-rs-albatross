use std::sync::atomic::Ordering;

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

    /// Returns our validator address.
    async fn get_address(&mut self) -> Result<Address, Self::Error> {
        Ok(self.validator.validator_address.read().clone())
    }

    /// Returns our validator signing key.
    async fn get_signing_key(&mut self) -> Result<String, Self::Error> {
        Ok(hex::encode(
            self.validator.signing_key.read().private.serialize_to_vec(),
        ))
    }

    /// Returns our validator voting key.
    async fn get_voting_key(&mut self) -> Result<String, Self::Error> {
        Ok(hex::encode(
            self.validator
                .voting_key
                .read()
                .secret_key
                .serialize_to_vec(),
        ))
    }

    async fn set_automatic_activation(
        &mut self,
        automatic_activation: bool,
    ) -> Result<bool, Self::Error> {
        self.validator
            .automatic_activate
            .store(automatic_activation, Ordering::Release);
        Ok(automatic_activation)
    }
}
