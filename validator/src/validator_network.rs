use std::collections::HashMap;
use std::collections::btree_map::BTreeMap;
use std::sync::{Arc, Weak};
use std::fmt;

use failure::Fail;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use block_albatross::{
    ForkProof, PbftProof, PbftProofBuilder, PbftProposal, PbftCommitMessage, PbftPrepareMessage,
    SignedPbftCommitMessage, SignedPbftPrepareMessage, SignedPbftProposal,
    SignedViewChange, ViewChange, ViewChangeProof, ViewChangeProofBuilder
};
use blockchain_albatross::{Blockchain, blockchain::BlockchainEvent};
use bls::bls12_381::{PublicKey, CompressedPublicKey};
use collections::grouped_list::Group;
use hash::{Blake2bHash, Hash};
use messages::Message;
use network::{Network, NetworkEvent, Peer};
use network_primitives::validator_info::{SignedValidatorInfo};
use network_primitives::address::PeerId;
use primitives::policy::{SLOTS, TWO_THIRD_SLOTS};
use primitives::validators::Validators;
use utils::mutable_once::MutableOnce;
use utils::observer::{PassThroughNotifier, weak_listener, weak_passthru_listener};
use handel::aggregation::AggregationEvent;
use handel::update::LevelUpdateMessage;

use crate::validator_agent::{ValidatorAgent, ValidatorAgentEvent};
use crate::validator_network::ValidatorNetworkError::NotInPbftPhase;
use crate::signature_aggregation::view_change::ViewChangeAggregation;
use crate::signature_aggregation::pbft::PbftAggregation;


#[derive(Clone, Debug, Fail)]
pub enum ValidatorNetworkError {
    #[fail(display = "pBFT already started with different proposal")]
    IncorrectProposal,
    #[fail(display = "Not in pBFT voting phase")]
    NotInPbftPhase
}

impl ValidatorNetworkError {
    fn is_minor(&self) -> bool {
        match self {
            NotInPbftPhase => true,
            _ => false
        }
    }
}

#[derive(Clone, Debug)]
pub enum ValidatorNetworkEvent {
    /// When a fork proof was given
    ForkProof(ForkProof),

    /// When a valid view change was completed
    ViewChangeComplete(ViewChange, ViewChangeProof),

    /// When a valid macro block is proposed by the correct pBFT-leader. This can happen multiple
    /// times during an epoch - i.e. when a proposal with a higher view number is received.
    PbftProposal(Blake2bHash, PbftProposal),

    /// When enough prepare signatures are collected for a proposed macro block
    PbftPrepareComplete(Blake2bHash, PbftProposal),

    /// When enough commit signatures from signers that also signed prepare messages are collected
    PbftCommitComplete(Blake2bHash, PbftProposal, PbftProof),
}


/// State of current pBFT phase
struct PbftState {
    /// The proposed macro block with justification
    proposal: PbftProposal,

    /// The hash of the header of the proposed macro block
    block_hash: Blake2bHash,

    /// The state of the signature aggregation for pBFT prepare and commit
    aggregation: PbftAggregation,
}

impl PbftState {
    pub fn new(block_hash: Blake2bHash, proposal: PbftProposal, node_id: usize, validators: Validators, peers: Arc<HashMap<usize, Arc<ValidatorAgent>>>) -> Self {
        let aggregation = PbftAggregation::new(block_hash.clone(), node_id, validators, peers, None);
        Self {
            proposal,
            block_hash,
            aggregation,
        }
    }
}

impl fmt::Debug for PbftState {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let (prepare_votes, commit_votes) = self.aggregation.votes();
        write!(f, "PbftState {{ proposal: {}, prepare: {}, commit: {}", self.block_hash, prepare_votes, commit_votes)
    }
}


#[derive(Debug, Default)]
struct ValidatorNetworkState {
    /// The peers that are connected that have the validator service flag set. So this is not
    /// exactly the set of validators. Potential validators should set this flag and then broadcast
    /// a `ValidatorInfo`.
    ///
    /// NOTE: The mapping from the `PeerId` is needed to efficiently remove agents from this set.
    /// NOTE: This becomes obsolete once we can actively connect to validators
    agents: HashMap<PeerId, Arc<ValidatorAgent>>,

    /// Peers for which we received a `ValidatorInfo` and thus have a BLS public key.
    potential_validators: BTreeMap<CompressedPublicKey, Arc<ValidatorAgent>>,

    /// Subset of validators that only includes validators that are active in the current epoch.
    /// NOTE: This is Arc'd, such that we can pass it to Handel without cloning.
    active_validators: Arc<HashMap<usize, Arc<ValidatorAgent>>>,

    /// Maps (view-change-number, block-number) to the proof that is being aggregated
    /// and a flag whether it's finalized. clear after macro block
    view_changes: HashMap<ViewChange, ViewChangeAggregation>,

    /// If we're in pBFT phase, this is the current state of it
    pbft_state: Option<PbftState>,
}


pub struct ValidatorNetwork {
    blockchain: Arc<Blockchain<'static>>,

    /// The signed validator info for this node
    info: SignedValidatorInfo,

    /// The validator network state
    state: RwLock<ValidatorNetworkState>,

    self_weak: MutableOnce<Weak<ValidatorNetwork>>,
    pub notifier: RwLock<PassThroughNotifier<'static, ValidatorNetworkEvent>>,
}

impl ValidatorNetwork {
    const MAX_VALIDATOR_INFOS: usize = 64;

    pub fn new(network: Arc<Network<Blockchain<'static>>>, blockchain: Arc<Blockchain<'static>>, info: SignedValidatorInfo) -> Arc<Self> {
        let this = Arc::new(ValidatorNetwork {
            blockchain,
            info,
            state: RwLock::new(ValidatorNetworkState::default()),
            self_weak: MutableOnce::new(Weak::new()),
            notifier: RwLock::new(PassThroughNotifier::new()),
        });

        Self::init_listeners(&this, network);

        this
    }

    fn init_listeners(this: &Arc<Self>, network: Arc<Network<Blockchain<'static>>>) {
        unsafe { this.self_weak.replace(Arc::downgrade(this)) };

        // register for peers joining and leaving
        network.notifier.write().register(weak_listener(Arc::downgrade(this), |this, event| {
            match event {
                NetworkEvent::PeerJoined(peer) => this.on_peer_joined(&peer),
                NetworkEvent::PeerLeft(peer) => this.on_peer_left(&peer),
                _ => {}
            }
        }));

        // register for finality
        this.blockchain.notifier.write().register(weak_listener(Arc::downgrade(this), |this, event| {
            match event {
                BlockchainEvent::Finalized => {
                    this.on_finality()
                },
                _ => {},
            }
        }));
    }

    fn on_peer_joined(&self, peer: &Arc<Peer>) {
        if peer.peer_address().services.is_validator() {
            let agent = ValidatorAgent::new(Arc::clone(peer), Arc::clone(&self.blockchain));

            // insert into set of all agents that have the validator service flag
            self.state.write().agents.insert(agent.peer_id(), Arc::clone(&agent));

            // register for messages received by agent
            agent.notifier.write().register(weak_passthru_listener(Weak::clone(&self.self_weak), |this, event| {
                match event {
                    ValidatorAgentEvent::ValidatorInfo(info) => {
                        this.on_validator_info(info);
                    },
                    ValidatorAgentEvent::ForkProof(fork_proof) => {
                        this.on_fork_proof(fork_proof);
                    }
                    ValidatorAgentEvent::ViewChange(update_message) => {
                        this.on_view_change_level_update(update_message)
                    },
                    ValidatorAgentEvent::PbftProposal(proposal) => {
                        this.on_pbft_proposal(proposal)
                    },
                    ValidatorAgentEvent::PbftPrepare(level_update) => {
                        this.on_pbft_prepare_level_update(level_update)
                    },
                    ValidatorAgentEvent::PbftCommit(level_update) => {
                        this.on_pbft_commit_level_update(level_update)
                    },
                }
            }));

            // send known validator infos to peer
            let mut infos = self.state.read().agents.iter()
                .filter_map(|(_, agent)| {
                    agent.state.read().validator_info.clone()
                })
                .take(Self::MAX_VALIDATOR_INFOS) // limit the number of validator infos
                .collect::<Vec<SignedValidatorInfo>>();
            infos.push(self.info.clone()); // add our infos
            peer.channel.send_or_close(Message::ValidatorInfo(infos));
        }
    }

    fn on_peer_left(&self, peer: &Arc<Peer>) {
        let mut state = self.state.write();

        if let Some(agent) = state.agents.remove(&peer.peer_address().peer_id) {
            info!("Validator left: {}", agent.peer_id());
        }
    }

    /// NOTE: assumes that the signature of the validator info was checked
    fn on_validator_info(&self, info: SignedValidatorInfo) {
        let mut state = self.state.write();

        trace!("Validator info: {:?}", info.message);

        if let Some(agent) = state.agents.get(&info.message.peer_address.peer_id) {
            let agent = Arc::clone(&agent);
            let agent_state = agent.state.upgradable_read();

            if let Some(current_info) = &agent_state.validator_info {
                if current_info.message.public_key == info.message.public_key {
                    // didn't change, do nothing
                    return;
                }
            }

            // insert into potential validators, indexed by compressed public key
            state.potential_validators.insert(info.message.public_key.clone(), Arc::clone(&agent));

            // check if active validator and put into `active` list
            // TODO: Use a HashMap to map PublicKeys to validator ID
            /*for (id, Group(_, public_key)) in self.blockchain.current_validators().groups().iter().enumerate() {
                if *public_key.compressed() == info.message.public_key {
                    trace!("Validator is active");
                    state.active_validators.insert(id, agent);
                    break;
                }
            }*/

            // put validator info into agent
            RwLockUpgradableReadGuard::upgrade(agent_state).validator_info = Some(info);
        }
        else {
            warn!("ValidatorInfo for unknown peer: {:?}", info);
        }
    }

    fn on_fork_proof(&self, fork_proof: ForkProof) {
        self.notifier.read().notify(ValidatorNetworkEvent::ForkProof(fork_proof.clone()));
        self.broadcast_fork_proof(fork_proof);
    }

    /// Called when we reach finality - i.e. when a macro block was produced
    /// TODO: Remove validators with keys that don't have stake anymore?
    fn on_finality(&self) {
        debug!("Clearing view change and pBFT proof");
        let mut state = self.state.write();

        // clear view changes
        state.view_changes.clear();

        // reset pBFT state
        state.pbft_state = None;

        // create mapping from validator ID to agent/peer
        let validators = self.blockchain.current_validators();
        let mut active_validators = HashMap::new();
        for (id, Group(n, public_key)) in validators.iter_groups().enumerate() {
            if let Some(agent) = state.potential_validators.get(public_key.compressed()) {
                debug!("Validator {}: {}", id, agent.validator_info().unwrap().peer_address.as_uri());
                active_validators.insert(id, Arc::clone(agent));
            }
            else {
                error!("Unreachable validator: {}: {:?}", id, public_key);
            }
        }
        state.active_validators = Arc::new(active_validators);
    }

    /// Pushes the update to the signature aggregation for this view-change
    fn on_view_change_level_update(&self, update_message: LevelUpdateMessage<ViewChange>) {
        //debug!("Received view change level update: {:?}", update_message);
        let state = self.state.read();

        if let Some(aggregation) = state.view_changes.get(&update_message.tag) {
            //debug!("Pushing it into aggregation: {:?}", aggregation);
            aggregation.push_update(update_message);
        }
    }

    /// Start pBFT with the given proposal.
    /// Either we generated that proposal, or we received it
    /// TODO: Don't forget to relay the proposal
    /// NOTE: Assumes the proposal was verified already
    pub fn on_pbft_proposal(&self, signed_proposal: SignedPbftProposal) {
        let mut state = self.state.write();


        let proposal = signed_proposal.message.clone();
        let signer_idx = signed_proposal.signer_idx as usize;
        let block_hash = proposal.header.hash::<Blake2bHash>();
        let active_validators = Arc::clone(&state.active_validators);

        if state.pbft_state.is_none() {
            state.pbft_state = Some(PbftState::new(
                block_hash.clone(),
                proposal,
                signer_idx,
                self.blockchain.current_validators().clone(),
                active_validators,
            ));

            drop(state);
        }
        else {
            // TODO
            unimplemented!("pBFT proposal already exists");
        }
    }

    pub fn on_pbft_prepare_level_update(&self, level_update: LevelUpdateMessage<PbftPrepareMessage>) {
        let mut state = self.state.write();

        if let Some(pbft) = &state.pbft_state {
            pbft.aggregation.push_prepare_level_update(level_update);
        }
        else {
            warn!("Not in pBFT phase, but received prepare update: {:?}", level_update);
        }
    }

    pub fn on_pbft_commit_level_update(&self, level_update: LevelUpdateMessage<PbftCommitMessage>) {
        // TODO: This is almost identical to the prepare one, maybe we can make the method generic over it?
        let mut state = self.state.write();

        if let Some(pbft) = &state.pbft_state {
            pbft.aggregation.push_commit_level_update(level_update);
        }
        else {
            warn!("Not in pBFT phase, but received prepare update: {:?}", level_update);
        }
    }

    // Public interface: start view changes, pBFT phase, push contributions

    /// Starts a new view-change
    pub fn start_view_change(&self, signed_view_change: SignedViewChange) {
        let view_change = signed_view_change.message.clone();
        let mut state = self.state.write();

        if let Some(aggregation) = state.view_changes.get(&view_change) {
            warn!("{:?} already exists with {} votes", signed_view_change.message, aggregation.votes());
            unimplemented!("We should add our signature to it, if we haven't already")
        }
        else {
            let validators = self.blockchain.current_validators().clone();

            // create view change aggregation
            let aggregation = ViewChangeAggregation::new(view_change.clone(), signed_view_change.signer_idx as usize, validators, Arc::clone(&state.active_validators), None);

            // register handler for when done and start (or use Future)
            {
                let view_change = view_change.clone();
                aggregation.inner.notifier.write().register(weak_passthru_listener(Weak::clone(&self.self_weak), move |this, event| {
                    match event {
                        AggregationEvent::Complete { best } => {
                            info!("Complete: {:?}", view_change);
                            let proof = ViewChangeProof::new(best.signature, best.signers);
                            this.notifier.read()
                                .notify(ValidatorNetworkEvent::ViewChangeComplete(view_change.clone(), proof))
                        }
                    }
                }));
            }

            // push our contribution
            aggregation.push_contribution(signed_view_change);

            state.view_changes.insert(view_change, aggregation);
        }
    }

    /// Start pBFT phase with our proposal
    pub fn start_pbft(&self, signed_proposal: SignedPbftProposal) {
        self.on_pbft_proposal(signed_proposal);
    }

    pub fn push_prepare(&self, signed_prepare: SignedPbftPrepareMessage) {
        let state = self.state.read();
        let pbft = state.pbft_state.as_ref().expect("Pushed pBFT prepare while not in pBFT phase");
        pbft.aggregation.push_signed_prepare(signed_prepare);
    }

    pub fn push_commit(&self, signed_commit: SignedPbftCommitMessage) {
        let state = self.state.read();
        let pbft = state.pbft_state.as_ref().expect("Pushed pBFT commit while not in pBFT phase");
        pbft.aggregation.push_signed_commit(signed_commit);
    }

    // Legacy broadcast methods -------------------------
    //
    // These are still used to relay `ValidatorInfo` and `PbftProposal`

    /// Broadcast to all known active validators
    fn broadcast_active(&self, msg: Message) {
        // FIXME: Active validators don't actively connect to other active validators right now.
        /*trace!("Broadcast to active validators: {:#?}", msg);
        for (_, agent) in self.state.read().active.iter() {
            agent.read().peer.channel.send_or_close(msg.clone())
        }*/
        self.broadcast_all(msg);
    }

    /// Broadcast to all known validators
    fn broadcast_all(&self, msg: Message) {
        trace!("Broadcast to all validators: {}", msg.ty());
        for (_, agent) in self.state.read().potential_validators.iter() {
            trace!("Sending to {}", agent.peer.peer_address());
            agent.peer.channel.send_or_close(msg.clone());
        }
    }

    /// Broadcast our own validator info
    fn broadcast_validator_info(&self, info: SignedValidatorInfo) {
        self.broadcast_all(Message::ValidatorInfo(vec![info]));
    }

    /// Broadcast pBFT proposal
    fn broadcast_pbft_proposal(&self, proposal: SignedPbftProposal) {
        self.broadcast_active(Message::PbftProposal(Box::new(proposal)));
    }

    /// Broadcast fork-proof
    fn broadcast_fork_proof(&self, fork_proof: ForkProof) {
        self.broadcast_active(Message::ForkProof(Box::new(fork_proof)));
    }
}
