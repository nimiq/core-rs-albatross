use std::collections::{HashMap, BTreeMap, BTreeSet};
use std::sync::Arc;

use bls::bls12_381::CompressedPublicKey;
use bls::bls12_381::lazy::LazyPublicKey;
use network_primitives::validator_info::SignedValidatorInfo;
use primitives::validators::Validators;
use blockchain_albatross::Blockchain;
use network::Network;

use crate::validator_agent::ValidatorAgent;


pub struct ValidatorPool {
    /// Reference to the `ConnectionPool` in order to establish connections
    network: Arc<Network<Blockchain<'static>>>,

    /// Blacklist of validators we don't want to connect to
    blacklist: BTreeSet<CompressedPublicKey>,

    /// The signed validator infos we received for other validators.
    /// This will be sent to newly connected validators
    infos: BTreeMap<CompressedPublicKey, SignedValidatorInfo>,

    /// The currently active validators.
    /// Used to match a validator's public key to an validator ID, when an active validator rejoins
    validator_id_by_pubkey: BTreeMap<CompressedPublicKey, usize>,

    /// Peers for which we received a `ValidatorInfo` and thus have a BLS public key.
    potential_validators: BTreeMap<CompressedPublicKey, Arc<ValidatorAgent>>,

    /// Subset of validators that only includes validators that are active in the current epoch.
    active_validator_agents: HashMap<usize, Arc<ValidatorAgent>>,

    /// Public keys and weights of active validators
    active_validators: Validators,
}


impl ValidatorPool {
    pub fn new(network: Arc<Network<Blockchain<'static>>>) -> Self {
        ValidatorPool {
            network,
            blacklist: BTreeSet::new(),
            infos: BTreeMap::new(),
            validator_id_by_pubkey: BTreeMap::new(),
            potential_validators: BTreeMap::new(),
            active_validator_agents: HashMap::new(),
            active_validators: Validators::empty(),
        }
    }

    pub fn blacklist(&mut self, pubkey: CompressedPublicKey) {
        self.blacklist.insert(pubkey);
    }

    pub fn reset_epoch(&mut self, validators: &Validators) {
        // clear data of last epoch
        self.validator_id_by_pubkey.clear();
        self.active_validator_agents.clear();
        self.active_validators = validators.clone();

        for (validator_id, validator) in validators.iter_groups().enumerate() {
            // create mapping from public key to validator ID
            let pubkey = validator.1.compressed();
            self.validator_id_by_pubkey.insert(pubkey.clone(), validator_id);

            if let Some(validator) = self.potential_validators.get(pubkey) {
                // if we already know the validator as potential validator, put into active validators
                self.active_validator_agents.insert(validator_id, Arc::clone(validator));
            }
            else if !self.blacklist.contains(pubkey) {
                // otherwise we'll try to connect to them, if we have a validator info
                if let Some(info) = self.infos.get(pubkey) {
                    let peer_address = Arc::new(info.message.peer_address.clone());
                    debug!("Trying to connect to: {}", peer_address);
                    if !self.network.connections.connect_outbound(Arc::clone(&peer_address)) {
                        warn!("Failed to connect to {}", peer_address);
                    }
                }
            }
        }
    }

    /// Called when we receive a validator info. If we are connected to the validator, the agent is
    /// passed along aswell.
    /// This returns true if the info is new (and thus the `ValidatorNetwork` should relay it to other validators.
    pub fn on_validator_info(&mut self, info: &SignedValidatorInfo, agent_opt: Option<Arc<ValidatorAgent>>) -> bool {
        let pubkey = &info.message.public_key;

        // if we were given the agent, we can update `potential_validators` and `active_validators`
        if let Some(agent) = agent_opt {
            // add to potential validators
            self.potential_validators.insert(pubkey.clone(), Arc::clone(&agent));

            // check if validator is active, and if so, add to active validators
            if let Some(&validator_id) = self.validator_id_by_pubkey.get(pubkey) {
                self.active_validator_agents.insert(validator_id, Arc::clone(&agent));
            }
        }

        // check if we have a validator info for the same public key
        if let Some(known_info) = self.infos.get(pubkey) {
            // if the validator info is older that the one we have, abort
            if info.message.valid_from <= known_info.message.valid_from {
                trace!("Received old validator info (newest valid_from={}): {:?}", known_info.message.valid_from, info);
                return false;
            }
        }

        // remember validator info
        self.infos.insert(pubkey.clone(), info.clone());

        true
    }

    /// Called when a connected validator peer disconnects
    pub fn on_validator_left(&mut self, agent: Arc<ValidatorAgent>) {
        let agent_state = agent.state.read();
        if let Some(info) = &agent_state.validator_info {
            let pubkey = &info.message.public_key;

            // remove from potential validators
            self.potential_validators.remove(pubkey);

            // remove from active validators, if possible
            if let Some(&validator_id) = self.validator_id_by_pubkey.get(pubkey) {
                self.active_validator_agents.remove(&validator_id);
            }
        }
    }

    pub fn get_potential_validator_agent(&self, pubkey: &CompressedPublicKey) -> Option<Arc<ValidatorAgent>> {
        self.potential_validators.get(pubkey).cloned()
    }

    pub fn get_active_validator_agent(&self, validator_id: usize) -> Option<Arc<ValidatorAgent>> {
        self.active_validator_agents.get(&validator_id).cloned()
    }

    pub fn iter_potential<'a>(&'a self) -> impl Iterator<Item=Arc<ValidatorAgent>> + 'a {
        self.potential_validators.iter()
            .map(|(_, agent)| Arc::clone(&agent))
    }

    pub fn iter_active<'a>(&'a self) -> impl Iterator<Item=Arc<ValidatorAgent>> + 'a {
        self.active_validator_agents.iter()
            .map(|(_, agent)| Arc::clone(&agent))
    }

    pub fn get_public_key(&self, validator_id: usize) -> Option<(&LazyPublicKey, usize)> {
        self.active_validators.get(validator_id)
            .map(|g| (&g.1, g.0 as usize))
    }

    pub fn active_validator_count(&self) -> usize {
        self.active_validators.num_groups()
    }
}
