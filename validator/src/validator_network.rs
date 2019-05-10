use std::sync::{Arc, Weak};
use network::{Network, Peer, NetworkEvent};
use std::collections::HashMap;
use parking_lot::RwLock;
use crate::validator_agent::{ValidatorAgent, ValidatorAgentEvent};
use network_primitives::validator_info::{ValidatorInfo, ValidatorId, SignedValidatorInfo};
use network_primitives::address::PeerId;
use utils::observer::{PassThroughNotifier, weak_passthru_listener, weak_listener};
use messages::Message;
use std::collections::btree_map::BTreeMap;
use block_albatross::{
    ViewChange, SignedViewChange, ViewChangeProof,
    SignedPbftPrepareMessage, SignedPbftCommitMessage, PbftProof,
    MacroHeader, SignedPbftProposal, PbftProposal
};
use blockchain_albatross::blockchain::Blockchain;
use bls::bls12_381::PublicKey;
use hash::{Blake2bHash, Hash};
use primitives::policy::TWO_THIRD_VALIDATORS;
use failure::Fail;


#[derive(Clone, Debug, Fail)]
pub enum ValidatorNetworkError {
    #[fail(display = "pBFT already started with different proposal")]
    IncorrectProposal,
    #[fail(display = "Not in pBFT voting phase")]
    NotInPbftPhase
}

pub enum ValidatorNetworkEvent {
    /// When a valid view change was completed
    ViewChangeComplete(ViewChange),

    /// When a valid macro block is proposed by the correct pBFT-leader. This can happen multiple
    /// times during an epoch - i.e. when a proposal with a higher view number is received.
    PbftProposal(PbftProposal),

    /// When enough prepare signatures are collected for a proposed macro block
    PbftPrepareComplete(Blake2bHash),

    /// When enough commit signatures from signers that also signed prepare messages are collected
    PbftCommitComplete(Blake2bHash),
}

pub struct ValidatorNetwork {
    network: Arc<Network>,
    blockchain: Arc<Blockchain<'static>>,

    /// The peers that are connected that have the validator service flag set. So this is not
    /// exactly the set of validators. Potential validators should set this flag and then broadcast
    /// a `ValidatorInfo`.
    agents: HashMap<PeerId, Arc<RwLock<ValidatorAgent>>>,

    /// Peers for which we received a `ValidatorInfo` and thus have a BLS public key
    validators: HashMap<ValidatorId, Arc<RwLock<ValidatorAgent>>>,

    /// Subset of validators that only includes validators that are active in the current epoch.
    active: HashMap<ValidatorId, Arc<RwLock<ValidatorAgent>>>,

    /// maps (view-change-number, block-number) to the proof that is being aggregated
    /// clear after macro block
    view_changes: BTreeMap<ViewChange, ViewChangeProof>,

    /// The current proposed macro header and pbft proof.
    ///
    /// This exists between proposal and macro block finalized. The header hash is stored for
    /// efficiency reasons.
    pbft_proof: Option<(PbftProposal, Blake2bHash, PbftProof)>,

    self_weak: Weak<RwLock<ValidatorNetwork>>,
    // XXX We might need recursive read locks.
    //  if a call to `commit_pbft_prepare` from the validator causes a `PbftPrepareComplete` that
    //  will call into the validator again to produce a `SignedPbftCommitMessage` and call
    //  `commit_pbft_commit`, which could cause a `PbftCommitComplete`. This will acquire multiple
    //  read locks of the notifier from the same thread.
    pub notifier: RwLock<PassThroughNotifier<'static, ValidatorNetworkEvent>>,
}

impl ValidatorNetwork {
    pub fn new(network: Arc<Network>, blockchain: Arc<Blockchain<'static>>) -> Arc<RwLock<Self>> {
        let this = Arc::new(RwLock::new(ValidatorNetwork {
            network,
            blockchain,
            agents: HashMap::new(),
            validators: HashMap::new(),
            active: HashMap::new(),
            view_changes: BTreeMap::new(),
            pbft_proof: None,
            self_weak: Weak::new(),
            notifier: RwLock::new(PassThroughNotifier::new()),
        }));

        ValidatorNetwork::init_listeners(&this);
        this
    }

    fn init_listeners(this: &Arc<RwLock<ValidatorNetwork>>) {
        this.write().self_weak = Arc::downgrade(this);
        let weak = Arc::downgrade(this);

        this.read().network.notifier.write().register(weak_listener(Arc::downgrade(this), |this, event| {
            match event {
                NetworkEvent::PeerJoined(peer) => this.write().on_peer_joined(&peer),
                NetworkEvent::PeerLeft(peer) => this.write().on_peer_left(&peer),
                _ => {}
            }
        }));
    }

    fn on_peer_joined(&mut self, peer: &Arc<Peer>) {
        if peer.peer_address().services.is_validator() {
            let agent = ValidatorAgent::new(Arc::clone(peer), Arc::clone(&self.blockchain));

            // insert into set of all agents that have the validator service flag
            self.agents.insert(peer.peer_address().peer_id.clone(), Arc::clone(&agent));

            // register for messages received by agent
            agent.read().notifier.write().register(weak_passthru_listener(Weak::clone(&self.self_weak), |this, event| {
                match event {
                    ValidatorAgentEvent::ValidatorInfo(info) => {
                        this.write().on_validator_info(info);
                    },
                    ValidatorAgentEvent::ViewChange { view_change, public_key, slots } => {
                        this.write().commit_view_change(view_change, &public_key, slots);
                    },
                    ValidatorAgentEvent::PbftProposal(proposal) => {
                        this.write().commit_pbft_proposal(proposal);
                    },
                    ValidatorAgentEvent::PbftPrepare { prepare, public_key, slots } => {
                        this.write().commit_pbft_prepare(prepare, &public_key, slots);
                    },
                    ValidatorAgentEvent::PbftCommit { commit, public_key, slots } => {
                        this.write().commit_pbft_commit(commit, &public_key, slots);
                    },
                }
            }));

            // send known validator infos to peer
            let infos = self.validators.iter()
                .filter_map(|(_, validator)| {
                    validator.read().validator_info.clone()
                }).collect::<Vec<SignedValidatorInfo>>();
            peer.channel.send_or_close(Message::ValidatorInfo(infos));
        }
    }

    fn on_peer_left(&mut self, peer: &Arc<Peer>) {
        if let Some(agent) = self.agents.remove(&peer.peer_address().peer_id) {
            if let Some(info) = &agent.read().validator_info {
                self.validators.remove(&info.message.validator_id);
                self.active.remove(&info.message.validator_id);
            }
        }
    }

    fn on_validator_info(&mut self, info: SignedValidatorInfo) {
        if let Some(agent) = self.agents.get(&info.message.peer_address.peer_id) {
            if let Some(current_info) = &agent.read().validator_info {
                if current_info.message.validator_id == info.message.validator_id {
                    // didn't change, do nothing
                    return;
                }

                // if the validator ID changed, remove peer from validator agents first
                self.validators.remove(&current_info.message.validator_id);
            }

            // add peer to validator agents
            self.validators.insert(info.message.validator_id.clone(), Arc::clone(agent));

            // TODO: check if active validator and put into `active` list

            // put validator info into agent
            agent.write().validator_info = Some(info);
        }
        else {
            debug!("ValidatorInfo for unknown peer: {:?}", info);
        }
    }

    /// Called when we reach finality - i.e. when a macro block was produced
    // TODO: register in consensus for macro blocks
    fn on_finality(&mut self) {
        debug!("Clearing view change and pBFT proof");
        self.view_changes.clear();
        self.pbft_proof = None;
        // TODO: Remove validators with keys that don't have stake anymore?
        // TODO: Compute set of validator agents that are now active.
        self.active.clear();
    }

    /// Commit a view change to the proofs being build and relay it if it's new
    pub fn commit_view_change(&mut self, view_change: SignedViewChange, public_key: &PublicKey, slots: u16) {
        // get the proof with the specific block number and view change number
        // if it doesn't exist, create a new one.
        let proof = self.view_changes.entry(view_change.message.clone())
            .or_insert_with(|| ViewChangeProof::new());

        // Aggregate signature - if it wasn't included yet, relay it
        if proof.add_signature(&public_key, slots, &view_change) {
            // if we have enough signatures, notify listeners
            if proof.verify(&view_change.message, TWO_THIRD_VALIDATORS) {
                self.notifier.read()
                    .notify(ValidatorNetworkEvent::ViewChangeComplete(view_change.message.clone()))
            }

            // broadcast new view change signature
            self.broadcast_active(Message::ViewChange(Box::new(view_change.clone())));
        }
    }

    /// Commit a macro block proposal
    pub fn commit_pbft_proposal(&mut self, proposal: SignedPbftProposal) {
        let commit = if let Some((current_proposal, _, proof)) = &self.pbft_proof {
            if *current_proposal == proposal.message {
                // if we already know the proposal, ignore it
                false
            }
            else if proposal.message.view_number <= current_proposal.view_number {
                // if it has a lower view number than the current one, ignore it
                debug!("Ignoring new macro block proposal with lower view change: {:#?}", proposal.message);
                false
            }
            else {
                // if it has a higher view number, commit it
                debug!("New macro block proposal with higher view change: {:#?}", proposal.message);
                true
            }
        }
        else {
            // if we don't have a proposal yet, commit it
            debug!("New macro block proposal: {:#?}", proposal.message);
            true
        };

        if commit {
            // remember proposal
            let block_hash = proposal.message.header.hash::<Blake2bHash>();
            self.pbft_proof = Some((proposal.message.clone(), block_hash, PbftProof::new()));

            // notify Jeff, a.k.a notify `Validator`
            self.notifier.read().notify(ValidatorNetworkEvent::PbftProposal(proposal.message.clone()));

            // relay proposal
            self.broadcast_active(Message::PbftProposal(Box::new(proposal)));
        }
    }

    /// Commit a pBFT prepare
    pub fn commit_pbft_prepare(&mut self, prepare: SignedPbftPrepareMessage, public_key: &PublicKey, slots: u16) -> Result<(), ValidatorNetworkError> {
        if let Some((proposal, block_hash, proof)) = &mut self.pbft_proof {
            // check if this prepare is for our current proposed block
            if prepare.message.block_hash != *block_hash {
                debug!("Prepare for unknown block: {}", prepare.message.block_hash);
                return Err(ValidatorNetworkError::IncorrectProposal);
            }

            // aggregate prepare signature - if new, relay
            if proof.add_prepare_signature(&public_key, slots, &prepare) {
                // notify if we reach threshold on prepare to begin commit
                if proof.prepare.verify(&prepare.message, TWO_THIRD_VALIDATORS) {
                    self.notifier.read()
                        .notify(ValidatorNetworkEvent::PbftPrepareComplete(prepare.message.block_hash.clone().clone()))
                }

                // NOTE: It might happen that we receive the prepare message after the commit. So we have
                //       to verify here too.
                if proof.verify(prepare.message.block_hash.clone(), TWO_THIRD_VALIDATORS) {
                    self.notifier.read()
                        .notify(ValidatorNetworkEvent::PbftCommitComplete(prepare.message.block_hash.clone()))
                }

                // broadcast new pbft prepare signature
                self.broadcast_active(Message::PbftPrepare(Box::new(prepare)));
            }
            Ok(())
        }
        else {
            debug!("Not in pBFT phase");
            Err(ValidatorNetworkError::NotInPbftPhase)
        }
    }

    /// Commit a pBFT commit
    pub fn commit_pbft_commit(&mut self, commit: SignedPbftCommitMessage, public_key: &PublicKey, slots: u16) -> Result<(), ValidatorNetworkError> {
        if let Some((header, block_hash, proof)) = &mut self.pbft_proof {
            // check if this prepare is for our current proposed block
            if commit.message.block_hash != *block_hash {
                debug!("Prepare for unknown block: {}", block_hash);
                return Err(ValidatorNetworkError::IncorrectProposal);
            }

            // aggregate commit signature - if new, relay
            if proof.add_commit_signature(&public_key, slots, &commit) {
                if proof.verify(commit.message.block_hash.clone(), TWO_THIRD_VALIDATORS) {
                    self.notifier.read()
                        .notify(ValidatorNetworkEvent::PbftCommitComplete(commit.message.block_hash.clone()))
                }

                // broadcast new pbft commit signature
                self.broadcast_active(Message::PbftCommit(Box::new(commit)))
            }
            Ok(())
        }
        else {
            debug!("Not in pBFT phase");
            Err(ValidatorNetworkError::NotInPbftPhase)
        }
    }

    /// Broadcast to all known active validators
    pub fn broadcast_active(&self, msg: Message) {
        for (_, agent) in self.active.iter() {
            agent.read().peer.channel.send_or_close(msg.clone())
        }
    }

    /// Broadcast to all known validators
    pub fn broadcast_all(&self, msg: Message) {
        for (_, agent) in self.validators.iter() {
            agent.read().peer.channel.send_or_close(msg.clone());
        }
    }

    /// Broadcast our own validator info
    pub fn broadcast_info(&self, info: SignedValidatorInfo) {
        self.broadcast_all(Message::ValidatorInfo(vec![info]));
    }

    pub fn get_view_change_proof(&self, view_change: &ViewChange) -> Option<&ViewChangeProof> {
        self.view_changes.get(view_change)
    }

    pub fn get_pbft_proposal(&self) -> Option<&PbftProposal> {
        self.pbft_proof.as_ref().map(|p| &p.0)
    }

    pub fn get_pbft_proposal_hash(&self) -> Option<&Blake2bHash> {
        self.pbft_proof.as_ref().map(|p| &p.1)
    }

    pub fn get_pbft_proof(&self) -> Option<&PbftProof> {
        self.pbft_proof.as_ref().map(|p| &p.2)
    }
}
