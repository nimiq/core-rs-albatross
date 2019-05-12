// use std::collections::HashMap;
// use std::ops::Deref;
use std::sync::{Arc, Weak};
// use std::time::Duration;

use parking_lot::RwLock;
// use rand::seq::SliceRandom;
// use rand::thread_rng;

use consensus::{Consensus, ConsensusEvent};

// use blockchain::{Blockchain, BlockchainEvent};
use database::Environment;
use bls::bls12_381::SecretKey;
use mempool::{Mempool, MempoolEvent, MempoolConfig};
use network::{Network, NetworkConfig, NetworkEvent, Peer};
use network_primitives::networks::NetworkId;
// use network_primitives::time::NetworkTime;
// use transaction::Transaction;
use utils::key_store::{Error as KeyStoreError, KeyStore};
use utils::mutable_once::MutableOnce;
// use utils::observer::Notifier;
// use utils::timers::Timers;

// use crate::consensus_agent::ConsensusAgent;
use crate::validator_network::ValidatorNetwork;
use crate::error::Error;

pub struct Validator {
    consensus: Arc<Consensus>,
    validator_network: Arc<ValidatorNetwork>,
    validator_key: SecretKey,

    state: RwLock<ValidatorState>,

    self_weak: MutableOnce<Weak<Validator>>,
}

pub struct ValidatorState {}

impl Validator {
    pub fn new(env: &'static Environment, network_id: NetworkId, network_config: NetworkConfig, mempool_config: MempoolConfig) -> Result<Arc<Self>, Error> {
        let consensus = Consensus::new(env, network_id, network_config, mempool_config)?;
        let validator_network = ValidatorNetwork::new(Arc::clone(&consensus.network), /*Arc::clone(&consensus.blockchain)*/ unimplemented!());


        let key_store = KeyStore::new("validator_key.db".to_string());

        let validator_key = match key_store.load_key() {
            Err(KeyStoreError::IoError(_)) => {
                let secret_key = SecretKey::generate(&mut rand::thread_rng());
                key_store.save_key(&secret_key)?;
                Ok(secret_key)
            },
            res => res,
        }?;

        let this = Arc::new(Validator {
            consensus,
            validator_network,
            validator_key,

            state: RwLock::new(ValidatorState {}),

            self_weak: MutableOnce::new(Weak::new()),
        });
        Validator::init_listeners(&this);
        Ok(this)
    }

    pub fn init_listeners(this: &Arc<Validator>) {
        unsafe { this.self_weak.replace(Arc::downgrade(this)) };

        let weak = Arc::downgrade(this);
        this.consensus.notifier.write().register(move |e: &ConsensusEvent| {
            let this = upgrade_weak!(weak);
            match e {
                ConsensusEvent::Established => this.on_consensus_established(),
                ConsensusEvent::Lost => this.on_consensus_lost(),
                _ => {}
            }
        });
    }

    pub fn on_consensus_established(&self) {
        unimplemented!();
    }

    pub fn on_consensus_lost(&self) {
        unimplemented!();
    }
}
