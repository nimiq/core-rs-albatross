use std::sync::{Arc, Weak};
use network::{Network, Peer, NetworkEvent};
use std::collections::HashMap;
use parking_lot::RwLock;
use crate::validator_agent::{ValidatorAgent, ValidatorAgentEvent};
use network_primitives::validator_info::ValidatorInfo;
use utils::observer::{PassThroughNotifier, weak_passthru_listener, weak_listener};
use messages::Message;
use std::collections::btree_map::BTreeMap;
use block_albatross::{
    ViewChange, SignedViewChange, ViewChangeProof,
    SignedPbftPrepareMessage, SignedPbftCommitMessage, PbftProof,
    MacroHeader, SignedPbftProposal, PbftProposal
};
use consensus::Consensus;
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
    validator_agents: HashMap<Arc<Peer>, Arc<RwLock<ValidatorAgent>>>,

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
    pub fn new(network: Arc<Network>, consensus: Arc<Consensus>) -> Arc<RwLock<Self>> {
        let this = Arc::new(RwLock::new(ValidatorNetwork {
            network,
            validator_agents: HashMap::new(),
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
            let agent = ValidatorAgent::new(Arc::clone(peer));
            self.validator_agents.insert(Arc::clone(peer), Arc::clone(&agent));

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
        }
    }

    fn on_peer_left(&mut self, peer: &Arc<Peer>) {
        self.validator_agents.remove(peer);
    }

    fn on_validator_info(&mut self, info: ValidatorInfo) {
        // TODO: make validator_agents a mapping from PeerAddress/Id to ValidatorAgent
        unimplemented!()
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

    // TODO: register in consensus for macro blocks
    pub fn on_finality(&mut self) {
        debug!("Clearing view change and pBFT proof");
        self.view_changes.clear();
        self.pbft_proof = None
    }

    fn broadcast_active(&self, msg: Message) {
        self.broadcast(msg, true);
    }

    fn broadcast(&self, msg: Message, active: bool) {
        for (peer, agent) in self.validator_agents.iter() {
            if !active || agent.read().is_active() {
                // TODO: in order to do this without cloning, we need to pass references all the way
                // to the websocket. We could easily just serialize a reference.
                peer.channel.send_or_close(msg.clone());
            }
        }
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
