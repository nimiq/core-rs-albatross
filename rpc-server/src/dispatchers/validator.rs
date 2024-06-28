use std::sync::atomic::Ordering;

use async_trait::async_trait;
use nimiq_consensus::ConsensusProxy;
use nimiq_keys::Address;
use nimiq_network_libp2p::Network;
use nimiq_rpc_interface::{types::RPCResult, validator::ValidatorInterface};
use nimiq_serde::Serialize;
use nimiq_validator::validator::ValidatorProxy;

use crate::error::Error;

pub struct ValidatorDispatcher {
    validator: ValidatorProxy,
    consensus: ConsensusProxy<Network>,
}

impl ValidatorDispatcher {
    pub fn new(validator: ValidatorProxy, consensus: ConsensusProxy<Network>) -> Self {
        ValidatorDispatcher {
            validator,
            consensus,
        }
    }
}

#[nimiq_jsonrpc_derive::service(rename_all = "camelCase")]
#[async_trait]
impl ValidatorInterface for ValidatorDispatcher {
    type Error = Error;

    async fn get_address(&mut self) -> RPCResult<Address, (), Self::Error> {
        Ok(self.validator.validator_address.read().clone().into())
    }

    async fn get_signing_key(&mut self) -> RPCResult<String, (), Self::Error> {
        Ok(hex::encode(self.validator.signing_key.read().private.serialize_to_vec()).into())
    }

    async fn get_voting_key(&mut self) -> RPCResult<String, (), Self::Error> {
        Ok(hex::encode(
            self.validator
                .voting_key
                .read()
                .secret_key
                .serialize_to_vec(),
        )
        .into())
    }

    async fn set_automatic_reactivation(
        &mut self,
        automatic_reactivate: bool,
    ) -> RPCResult<(), (), Self::Error> {
        self.validator
            .automatic_reactivate
            .store(automatic_reactivate, Ordering::Release);

        log::debug!("Automatic reactivation set to {}.", automatic_reactivate);
        Ok(().into())
    }

    async fn is_validator_elected(&mut self) -> RPCResult<bool, (), Self::Error> {
        let is_elected = self.validator.slot_band.read().is_some();
        Ok(is_elected.into())
    }

    async fn is_validator_synced(&mut self) -> RPCResult<bool, (), Self::Error> {
        let is_synced = self.consensus.is_ready_for_validation();
        Ok(is_synced.into())
    }
}
