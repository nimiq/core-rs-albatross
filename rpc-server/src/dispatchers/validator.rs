use std::{mem, sync::Arc};

use async_trait::async_trait;
use nimiq_bls::{KeyPair as BlsKeyPair, SecretKey as BlsSecretKey};
use nimiq_consensus::ConsensusProxy;
use nimiq_keys::Address;
use nimiq_network_libp2p::Network;
use nimiq_rpc_interface::{
    types::{RPCResult, ValidatorHealth},
    validator::ValidatorInterface,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_validator::{health::ValidatorHealthState, validator::ValidatorState};
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

    async fn get_address(&mut self) -> RPCResult<Address, (), Self::Error> {
        Ok(self.validator.read().validator_address.clone().into())
    }

    // TODO: why do we give out secret keys via RPC?
    async fn get_signing_key(&mut self) -> RPCResult<String, (), Self::Error> {
        Ok(hex::encode(self.validator.read().signing_key.private.serialize_to_vec()).into())
    }

    async fn get_voting_key(&mut self) -> RPCResult<String, (), Self::Error> {
        Ok(hex::encode(
            self.validator
                .read()
                .voting_keys
                .get_current_key()
                .secret_key
                .serialize_to_vec(),
        )
        .into())
    }

    async fn get_voting_keys(&mut self) -> RPCResult<Vec<String>, (), Self::Error> {
        Ok(self
            .validator
            .read()
            .voting_keys
            .get_keys()
            .into_iter()
            .map(|key| hex::encode(key.secret_key.serialize_to_vec()))
            .collect::<Vec<String>>()
            .into())
    }

    async fn get_validator_health(&mut self) -> RPCResult<ValidatorHealth, (), Self::Error> {
        Ok(match self.validator.read().validator_health.health() {
            ValidatorHealthState::Green => ValidatorHealth::Green.into(),
            ValidatorHealthState::Yellow => ValidatorHealth::Yellow.into(),
            ValidatorHealthState::Red => ValidatorHealth::Red.into(),
        })
    }

    async fn add_voting_key(&mut self, secret_key: String) -> RPCResult<(), (), Self::Error> {
        self.validator.write().voting_keys.add_key(BlsKeyPair::from(
            BlsSecretKey::deserialize_from_vec(&hex::decode(secret_key)?)?,
        ));
        Ok(().into())
    }

    async fn set_automatic_reactivation(
        &mut self,
        automatic_reactivate: bool,
    ) -> RPCResult<(), (), Self::Error> {
        let old = mem::replace(
            &mut self.validator.write().automatic_reactivate,
            automatic_reactivate,
        );

        log::debug!(
            "Automatic reactivation set to {} (from {}).",
            automatic_reactivate,
            old
        );
        Ok(().into())
    }

    async fn is_validator_elected(&mut self) -> RPCResult<bool, (), Self::Error> {
        let is_elected = self.validator.read().slot_band.is_some();
        Ok(is_elected.into())
    }

    async fn is_validator_synced(&mut self) -> RPCResult<bool, (), Self::Error> {
        let is_synced = self.consensus.is_ready_for_validation();
        Ok(is_synced.into())
    }
}
