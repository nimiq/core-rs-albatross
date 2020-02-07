use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use parking_lot::RwLock;

use blockchain_albatross::Blockchain;
use bls::lazy::LazyPublicKey;
use bls::CompressedPublicKey;
use hash::{Blake2bHash, Hash};
use network::Network;
use network_primitives::validator_info::{SignedValidatorInfo, ValidatorInfo};
use primitives::slot::{SlotBand, SlotCollection, ValidatorSlots};

use crate::validator_agent::ValidatorAgent;

pub enum PushResult {
    OldInfo,
    InvalidSignature,
    InvalidPublicKey,
    Added,
}

// TODO: Arc CompressedPublicKey?
pub struct ValidatorPool {
    /// Reference to the `ConnectionPool` in order to establish connections
    network: Arc<Network<Blockchain>>,

    /// Blacklist of validators we don't want to connect to
    blacklist: RwLock<BTreeSet<CompressedPublicKey>>,

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
    active_validators_slots: ValidatorSlots,
}

impl ValidatorPool {
    pub fn new(network: Arc<Network<Blockchain>>) -> Self {
        ValidatorPool {
            network,
            blacklist: RwLock::new(BTreeSet::new()),
            infos: BTreeMap::new(),
            validator_id_by_pubkey: BTreeMap::new(),
            potential_validators: BTreeMap::new(),
            active_validator_agents: HashMap::new(),
            active_validators_slots: ValidatorSlots::default(),
        }
    }

    pub fn blacklist(&self, pubkey: CompressedPublicKey) {
        self.blacklist.write().insert(pubkey);
    }

    pub fn reset_epoch(&mut self, validators: &ValidatorSlots) {
        // clear data of last epoch
        self.validator_id_by_pubkey.clear();
        self.active_validator_agents.clear();
        self.active_validators_slots = validators.clone();

        for (validator_id, validator) in validators.iter().enumerate() {
            // create mapping from public key to validator ID
            let pubkey = validator.public_key().compressed();
            self.validator_id_by_pubkey
                .insert(pubkey.clone(), validator_id);

            if let Some(validator) = self.potential_validators.get(pubkey) {
                // if we already know the validator as potential validator, put into active validators
                self.active_validator_agents
                    .insert(validator_id, Arc::clone(validator));
            } else if self.blacklist.read().contains(pubkey) {
                // ignore
            } else if let Some(info) = self.infos.get(pubkey) {
                // otherwise we'll try to connect to them, if we have a validator info
                self.connect_to_peer(&info.message);
            } else {
                warn!(
                    "No validator info for: {} ({} votes)",
                    validator_id,
                    validator.num_slots()
                );
            }
        }
    }

    /// Called when we receive a validator info. If we are connected to the validator, the agent is
    /// passed along as well.
    pub fn push_validator_info(&mut self, info: &SignedValidatorInfo) -> PushResult {
        let pubkey = &info.message.public_key;

        // check if we have a validator info for the same public key
        if let Some(known_info) = self.infos.get(pubkey) {
            // if the validator info is older that the one we have, abort
            if info.message.valid_from <= known_info.message.valid_from {
                trace!(
                    "Received old validator info (newest valid_from={}): {:?}",
                    known_info.message.valid_from,
                    info
                );
                return PushResult::OldInfo;
            }
        }

        // uncompress public key for signature verification
        // TODO: We might want to store this, or use LazyPublicKey
        let pub_key_uncompressed = match pubkey.uncompress() {
            Ok(pk) => pk,
            Err(_) => {
                warn!("Invalid public key in validator info: {:?}", info.message);
                return PushResult::InvalidPublicKey;
            }
        };

        // verify the signature of the validator info
        if !info.verify(&pub_key_uncompressed) {
            warn!("Invalid signature for validator info: {:?}", info.message);
            return PushResult::InvalidSignature;
        }

        // remember validator info
        self.infos.insert(pubkey.clone(), info.clone());

        debug!(
            "Added validator info for: {}: {}",
            pubkey.hash::<Blake2bHash>(),
            info.message.peer_address
        );

        PushResult::Added
    }

    pub fn connect_to_agent(&mut self, pubkey: &CompressedPublicKey, agent: &Arc<ValidatorAgent>) {
        if self.blacklist.read().contains(pubkey) {
            return;
        }

        debug!(
            "Connecting agent to validator: {}: {}",
            pubkey.hash::<Blake2bHash>(),
            agent.peer.peer_address()
        );

        // add to potential validators
        if self
            .potential_validators
            .insert(pubkey.clone(), Arc::clone(agent))
            .is_none()
        {
            // The agent for this validator public key was previously unknown, so we need to check
            // if they're active and insert them into our active validator map.
            if let Some(&validator_id) = self.validator_id_by_pubkey.get(pubkey) {
                self.active_validator_agents
                    .insert(validator_id, Arc::clone(&agent));
            }
        }
    }

    pub fn connect_to_peer(&self, info: &ValidatorInfo) {
        if self.blacklist.read().contains(&info.public_key) {
            return;
        }

        let peer_address = Arc::new(info.peer_address.clone());
        debug!("Trying to connect to: {}", peer_address);
        if !self
            .network
            .connections
            .connect_outbound(Arc::clone(&peer_address))
        {
            //warn!("Failed to connect to {}. Blacklist them for now (TODO).", peer_address);
            // TODO: Ideally the connection pool should handle this. Just don't try to reconnect to
            // them for a while
            //self.blacklist.write().insert(info.public_key.clone());
        }
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

    pub fn get_potential_validator_agent(
        &self,
        pubkey: &CompressedPublicKey,
    ) -> Option<Arc<ValidatorAgent>> {
        self.potential_validators.get(pubkey).cloned()
    }

    pub fn get_active_validator_agent(&self, validator_id: usize) -> Option<Arc<ValidatorAgent>> {
        self.active_validator_agents.get(&validator_id).cloned()
    }

    pub fn iter_potential<'a>(&'a self) -> impl Iterator<Item = Arc<ValidatorAgent>> + 'a {
        self.potential_validators
            .iter()
            .map(|(_, agent)| Arc::clone(&agent))
    }

    pub fn iter_active<'a>(&'a self) -> impl Iterator<Item = Arc<ValidatorAgent>> + 'a {
        self.active_validator_agents
            .iter()
            .map(|(_, agent)| Arc::clone(&agent))
    }

    pub fn get_public_key(&self, validator_id: usize) -> Option<&LazyPublicKey> {
        self.active_validators_slots
            .get_by_band_number(validator_id as u16)
            .map(|validator| validator.public_key())
    }

    pub fn get_num_slots(&self, validator_id: usize) -> Option<usize> {
        self.active_validators_slots
            .get_by_band_number(validator_id as u16)
            .map(|validator| validator.num_slots() as usize)
    }

    pub fn get_validator_info(&self, pubkey: &CompressedPublicKey) -> Option<&SignedValidatorInfo> {
        self.infos.get(pubkey)
    }

    pub fn active_validator_count(&self) -> usize {
        self.active_validators_slots.len()
    }

    pub fn iter_validator_infos(&self) -> impl Iterator<Item = &SignedValidatorInfo> {
        self.infos.iter().map(|(_, info)| info)
    }

    pub fn is_active(&self, pubkey: &CompressedPublicKey) -> bool {
        self.validator_id_by_pubkey.contains_key(&pubkey)
    }
}
