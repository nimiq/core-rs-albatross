use std::{mem, sync::Arc};

use async_trait::async_trait;
use nimiq_bls::{KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_consensus::ConsensusProxy;
use nimiq_keys::Address;
use nimiq_network_libp2p::Network;
use nimiq_rpc_interface::{types::RPCResult, validator::ValidatorInterface};
use nimiq_serde::Deserialize;
use nimiq_validator::validator::ValidatorState;
use parking_lot::RwLock;

use crate::error::Error;

pub struct ValidatorDispatcher {
    validator: Arc<RwLock<ValidatorState>>,
    consensus: ConsensusProxy<Network>,
}

impl ValidatorDispatcher {
    pub fn new(validator: Arc<RwLock<ValidatorState>>, consensus: ConsensusProxy<Network>) -> Self {
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

    async fn get_address(&self) -> RPCResult<Address, (), Self::Error> {
        Ok(self.validator.read().validator_address.clone().into())
    }

    async fn add_voting_key(&self, secret_key: String) -> RPCResult<(), (), Self::Error> {
        self.validator.write().voting_keys.add_key(BlsKeyPair::from(
            BlsSecretKey::deserialize_from_vec(&hex::decode(secret_key)?)?,
        ));
        Ok(().into())
    }

    async fn set_automatic_reactivation(
        &self,
        automatic_reactivate: bool,
    ) -> RPCResult<(), (), Self::Error> {
        let mut validator = self.validator.write();
        let old = mem::replace(&mut validator.automatic_reactivate, automatic_reactivate);

        log::debug!(
            "Automatic reactivation set to {} (from {}).",
            automatic_reactivate,
            old
        );
        Ok(().into())
    }

    async fn is_validator_elected(&self) -> RPCResult<bool, (), Self::Error> {
        let is_elected = self.validator.read().slot_band.is_some();
        Ok(is_elected.into())
    }

    async fn is_validator_synced(&self) -> RPCResult<bool, (), Self::Error> {
        let is_synced = self.consensus.is_ready_for_validation();
        Ok(is_synced.into())
    }
}
