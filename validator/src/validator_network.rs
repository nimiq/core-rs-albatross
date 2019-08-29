use std::collections::HashMap;
use std::collections::btree_map::BTreeMap;
use std::sync::{Arc, Weak};
use std::fmt;

use failure::Fail;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use block_albatross::{
    BlockHeader,
    ForkProof, PbftProof, PbftProofBuilder, PbftProposal,
    PbftPrepareMessage, PbftCommitMessage,
    SignedPbftCommitMessage, SignedPbftPrepareMessage, SignedPbftProposal,
    SignedViewChange, ViewChange, ViewChangeProof, ViewChangeProofBuilder
};
use block_albatross::signed::AggregateProof;
use blockchain_albatross::{Blockchain, blockchain::BlockchainEvent};
use bls::bls12_381::{PublicKey, CompressedPublicKey};
use collections::grouped_list::Group;
use hash::{Blake2bHash, Hash};
use messages::Message;
use network::{Network, NetworkEvent, Peer};
use network_primitives::validator_info::{SignedValidatorInfo};
use network_primitives::address::PeerId;
use primitives::policy::{SLOTS, TWO_THIRD_SLOTS, is_macro_block_at};
use primitives::validators::{Validators, IndexedSlot};
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
    #[fail(display = "View change already in progress: {:?}", _0)]
    ViewChangeAlreadyExists(ViewChange),
    #[fail(display = "pBFT phase already in progress: {:?}", _0)]
    PbftAlreadyStarted(PbftProposal),

    // TODO: check if those are still used
    #[fail(display = "Not in pBFT voting phase")]
    NotInPbftPhase,

    #[fail(display = "Unkown pBFT proposal")]
    UnknownProposal,
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

    /// When the pBFT proof is complete
    PbftComplete(Blake2bHash, PbftProposal, PbftProof),
}


/// State of current pBFT phase
struct PbftState {
    /// The proposed macro block with justification
    proposal: SignedPbftProposal,

    /// Whether the Proposal has been checked and if it's valid. The proposal might not have been
    /// validated if the block chain isn't complete up to the proposed macro block.
    proposal_valid: Option<bool>,

    /// The hash of the header of the proposed macro block
    block_hash: Blake2bHash,

    /// The state of the signature aggregation for pBFT prepare and commit
    aggregation: Arc<RwLock<PbftAggregation>>,

    /// The pBFT prepare proof, once it's complete
    prepare_proof: Option<AggregateProof<PbftPrepareMessage>>,
}

impl PbftState {
    pub fn new(block_hash: Blake2bHash, proposal: SignedPbftProposal, node_id: usize, validators: Validators, peers: Arc<HashMap<usize, Arc<ValidatorAgent>>>) -> Self {
        let aggregation = Arc::new(RwLock::new(PbftAggregation::new(block_hash.clone(), node_id, validators, peers, None)));
        Self {
            proposal,
            proposal_valid: None,
            block_hash,
            aggregation,
            prepare_proof: None,
        }
        // TODO: Call `update_verified` on this. But I'd rather do it outside of the constructor.
    }

    fn check_verified(&self, chain: &Blockchain, chain_height: u32) -> Option<bool> {
        let block_number = self.proposal.message.header.block_number;
        let view_number = self.proposal.message.header.view_number;

        // Can we verify validity of macro block?
        if let Some(IndexedSlot { slot, .. }) = chain.get_block_producer_at(block_number, view_number, None) {
            let public_key = &slot.public_key.uncompress_unchecked();

            // Check the validity of the block
            if let Err(e) = chain.verify_block_header(
                &BlockHeader::Macro(self.proposal.message.header.clone()),
                self.proposal.message.view_change.as_ref(),
                unimplemented!("Intended slot owner?"), unimplemented!("Would it make sense to pass a Read transaction?")) {
                debug!("[PBFT-PROPOSAL] Invalid macro block header: {:?}", e);
                return Some(false);
            }

            // Check the signer index
            if let Some(ref validator) = chain.get_current_validator_by_idx(self.proposal.signer_idx) {
                // Does the key own the current slot?
                if validator != slot.public_key {
                    return Some(false);
                }
            } else {
                // No validator at this index
                return Some(false);
            }

            // Check the signature of the proposal
            if !self.proposal.verify(&public_key) {
                debug!("[PBFT-PROPOSAL] Invalid signature");
                return Some(false);
            }

            Some(true)
        } else {
            // TODO: Limit this somehow
            if self.proposal.message.header.block_number > chain_height + 1 {
                None
            } else {
                warn!("Rejecting proposal: No such pBFT leader");
                Some(false)
            }
        }
    }

    // Returns true if verified is true for the first time
    pub fn update_verified(&mut self, blockchain: &Blockchain, chain_height: u32) -> Option<bool> {
        if self.proposal_valid.is_none() {
            self.proposal_valid = self.check_verified(blockchain, chain_height);
        }
        self.proposal_valid
    }
}

impl fmt::Debug for PbftState {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        let (prepare_votes, commit_votes) = self.aggregation.read().votes();
        write!(f, "PbftState {{ proposal: {}, prepare: {}, commit: {}", self.block_hash, prepare_votes, commit_votes)
    }
}

#[derive(Default)]
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
    pbft_states: BTreeMap<Blake2bHash, PbftState>,

    /// If we're an active validator, set our validator ID here
    validator_id: Option<usize>,
}


pub struct ValidatorNetwork {
    blockchain: Arc<Blockchain<'static>>,

    /// The signed validator info for this node
    info: SignedValidatorInfo,

    /// The validator network state
    state: RwLock<ValidatorNetworkState>,

    self_weak: MutableOnce<Weak<ValidatorNetwork>>,
    pub notifier: Arc<RwLock<PassThroughNotifier<'static, ValidatorNetworkEvent>>>,
}

impl ValidatorNetwork {
    const MAX_VALIDATOR_INFOS: usize = 64;

    pub fn new(network: Arc<Network<Blockchain<'static>>>, blockchain: Arc<Blockchain<'static>>, info: SignedValidatorInfo) -> Arc<Self> {
        let this = Arc::new(ValidatorNetwork {
            blockchain,
            info,
            state: RwLock::new(ValidatorNetworkState::default()),
            self_weak: MutableOnce::new(Weak::new()),
            notifier: Arc::new(RwLock::new(PassThroughNotifier::new())),
        });

        Self::init_listeners(&this, network);

        this
    }

    fn init_listeners(this: &Arc<Self>, network: Arc<Network<Blockchain<'static>>>) {
        unsafe { this.self_weak.replace(Arc::downgrade(this)) };

        // Register for peers joining and leaving
        network.notifier.write().register(weak_listener(Arc::downgrade(this), |this, event| {
            match event {
                NetworkEvent::PeerJoined(peer) => this.on_peer_joined(&peer),
                NetworkEvent::PeerLeft(peer) => this.on_peer_left(&peer),
                _ => {}
            }
        }));
    }

    fn on_peer_joined(&self, peer: &Arc<Peer>) {
        if peer.peer_address().services.is_validator() {
            let agent = ValidatorAgent::new(Arc::clone(peer), Arc::clone(&self.blockchain));

            // Insert into set of all agents that have the validator service flag
            self.state.write().agents.insert(agent.peer_id(), Arc::clone(&agent));

            // Register for messages received by agent
            agent.notifier.write().register(weak_passthru_listener(Weak::clone(&self.self_weak), |this, event| {
                match event {
                    ValidatorAgentEvent::ValidatorInfo(info) => {
                        this.on_validator_info(info);
                    },
                    ValidatorAgentEvent::ForkProof(fork_proof) => {
                        this.on_fork_proof(fork_proof);
                    }
                    ValidatorAgentEvent::ViewChange(update_message) => {
                        this.on_view_change_level_update(update_message);
                    },
                    ValidatorAgentEvent::PbftProposal(proposal) => {
                        this.on_pbft_proposal(proposal);
                    },
                    ValidatorAgentEvent::PbftPrepare(level_update) => {
                        this.on_pbft_prepare_level_update(level_update);
                    },
                    ValidatorAgentEvent::PbftCommit(level_update) => {
                        this.on_pbft_commit_level_update(level_update);
                    },
                }
            }));

            // Send known validator infos to peer
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
                    // Didn't change, do nothing
                    return;
                }
            }

            // Insert into potential validators, indexed by compressed public key
            state.potential_validators.insert(info.message.public_key.clone(), Arc::clone(&agent));

            // Check if active validator and put into `active` list
            // TODO: Use a HashMap to map PublicKeys to validator ID
            /*for (id, Group(_, public_key)) in self.blockchain.current_validators().groups().iter().enumerate() {
                if *public_key.compressed() == info.message.public_key {
                    trace!("Validator is active");
                    state.active_validators.insert(id, agent);
                    break;
                }
            }*/

            // Put validator info into agent
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

    /// Called when we reach finality - i.e. when a macro block was produced. This must be called be the
    /// validator.
    ///
    /// `validator_id`: The index of the validator (a.k.a `pk_idx`), if we're active
    pub fn on_finality(&self, validator_id: Option<usize>) {
        trace!("Clearing view change and pBFT proof");
        let mut state = self.state.write();

        // Clear view changes
        state.view_changes.clear();

        // Clear pBFT states
        state.pbft_states.clear();

        // Set validator ID
        state.validator_id = validator_id;

        // Create mapping from validator ID to agent/peer
        let validators = self.blockchain.current_validators();
        let mut active_validators = HashMap::new();
        for (id, Group(n, public_key)) in validators.iter_groups().enumerate() {
            if let Some(agent) = state.potential_validators.get(public_key.compressed()) {
                trace!("Validator {}: {}", id, agent.validator_info().unwrap().peer_address.as_uri());
                active_validators.insert(id, Arc::clone(agent));
            }
            else {
                error!("Unreachable validator: {}: {:?}", id, public_key);
            }
        }
        state.active_validators = Arc::new(active_validators);
    }

    /// Called when a new block is added
    pub fn on_blockchain_extended(&self) {
        let new_height = self.blockchain.block_number();

        // We only have to do this update if we actually got the last micro block of an epoch
        if is_macro_block_at(new_height + 1) {
            let mut state = self.state.write();
            let mut invalid = Vec::<Blake2bHash>::new();

            for (block_hash, pbft) in &mut state.pbft_states {
                match pbft.update_verified(&self.blockchain, new_height) {
                    None => return,  // Verification status didn't change
                    Some(false) => { // Invalid proposal
                        invalid.push(block_hash.clone());
                        break;
                    }
                    Some(true) => () // continue
                }

                debug!("Verified macro block proposal: {}", block_hash);
            }

            for key in invalid {
                debug!("Previously unconfirmed macro block proposal now confirmed invalid: {}", key);
                state.pbft_states.remove(&key);
            }
        }
    }

    /// Pushes the update to the signature aggregation for this view-change
    fn on_view_change_level_update(&self, update_message: LevelUpdateMessage<ViewChange>) {
        let state = self.state.read();
        if let Some(aggregation) = state.view_changes.get(&update_message.tag) {
            aggregation.push_update(update_message);
            debug!("View change: {}", fmt_vote_progress(aggregation.votes()));
        }
    }

    /// Start pBFT with the given proposal.
    /// Either we generated that proposal, or we received it
    pub fn on_pbft_proposal(&self, signed_proposal: SignedPbftProposal) -> Result<(), ValidatorNetworkError> {
        let mut state = self.state.write();
        let block_hash = signed_proposal.message.header.hash::<Blake2bHash>();

        if let Some(pbft) = state.pbft_states.get(&block_hash) {
            // Proposals might differ in view change
            if pbft.proposal.message.view_change != signed_proposal.message.view_change {
                error!("Proposal with different view change already exists: {:?} and {:?}", pbft.proposal.message.view_change, signed_proposal.message.view_change);
                return Err(ValidatorNetworkError::PbftAlreadyStarted(pbft.proposal.message.clone()))
            }
            // Proposal already exists, do nothing
        }
        else {
            let active_validators = Arc::clone(&state.active_validators);
            let validator_id = state.validator_id.expect("Not an active validator");

            debug!("pBFT proposal by validator {}: {:#?}", validator_id, signed_proposal);

            let pbft = PbftState::new(
                block_hash.clone(),
                signed_proposal.clone(),
                validator_id,
                self.blockchain.current_validators().clone(),
                active_validators,
            );

            // The prepare handler. This will store the finished prepare proof in the pBFT state
            let key = block_hash.clone();
            pbft.aggregation.read().prepare_aggregation.notifier.write()
                .register(weak_passthru_listener(Weak::clone(&self.self_weak), move |this, event| {
                    match event {
                        AggregationEvent::Complete { best } => {
                            let event = if let Some(pbft) = this.state.write().pbft_states.get_mut(&key) {
                                if pbft.prepare_proof.is_none() {
                                    // Build prepare proof
                                    let prepare_proof = AggregateProof::new(best.signature, best.signers);
                                    trace!("Prepare complete: {:?}", prepare_proof);
                                    pbft.prepare_proof = Some(prepare_proof);

                                    // Return the event
                                    Some(ValidatorNetworkEvent::PbftPrepareComplete(pbft.block_hash.clone(), pbft.proposal.message.clone()))
                                } else {
                                    warn!("Prepare proof already exists");
                                    None
                                }
                            } else {
                                error!("No pBFT state");
                                None
                            };
                            // If we generated a prepare complete event, notify the validator
                            event.map(move |event| this.notifier.read().notify(event));
                        }
                    }
                }));

            // The commit handler. This will store the finished commit proof and construct the
            // pBFT proof.
            let key = block_hash.clone();
            pbft.aggregation.read().commit_aggregation.notifier.write()
                .register(weak_passthru_listener(Weak::clone(&self.self_weak), move |this, event| {
                    match event {
                        AggregationEvent::Complete { best } => {
                            let event = if let Some(pbft) = this.state.write().pbft_states.get_mut(&key) {
                                // Build commit proof
                                let commit_proof = AggregateProof::new(best.signature, best.signers);
                                trace!("Commit complete: {:?}", commit_proof);

                                // NOTE: The commit evaluator will only mark the signature as final, if there are enough commit signatures from validators that also
                                //       signed prepare messages. Thus a complete prepare proof must exist at this point.
                                // NOTE: Either `take()` it, or `clone()` it. Really doesn't matter I guess
                                let prepare_proof = pbft.prepare_proof.take().expect("No pBFT prepare proof");
                                let pbft_proof = PbftProof { prepare: prepare_proof, commit: commit_proof };

                                // Return the event
                                Some(ValidatorNetworkEvent::PbftComplete(pbft.block_hash.clone(), pbft.proposal.message.clone(), pbft_proof))
                            } else {
                                error!("No pBFT state");
                                None
                            };
                            // If we generated a prepare complete event, notify the validatir
                            event.map(move |event| this.notifier.read().notify(event));
                        }
                    }
                }));

            // Add pBFT state
            state.pbft_states.insert(block_hash.clone(), pbft);

            // We need to drop the state before notifying and relaying
            drop(state);

            // Notify Validator
            self.notifier.read().notify(ValidatorNetworkEvent::PbftProposal(block_hash.clone(), signed_proposal.message.clone()));

            // Broadcast to other validators
            self.broadcast_pbft_proposal(signed_proposal);
        }

        Ok(())
    }

    pub fn on_pbft_prepare_level_update(&self, level_update: LevelUpdateMessage<PbftPrepareMessage>) {
        let mut state = self.state.write();

        if let Some(pbft) = state.pbft_states.get(&level_update.tag.block_hash) {
            let aggregation = Arc::clone(&pbft.aggregation);
            let aggregation = aggregation.read();
            drop(state);
            aggregation.push_prepare_level_update(level_update);
            let (prepare_votes, commit_votes) = aggregation.votes();
            debug!("pBFT: Prepare: {}, Commit: {}", fmt_vote_progress(prepare_votes), fmt_vote_progress(commit_votes));
        }
        else {
            warn!("Not in pBFT phase, but received prepare update: {:?}", level_update);
        }
    }

    pub fn on_pbft_commit_level_update(&self, level_update: LevelUpdateMessage<PbftCommitMessage>) {
        // TODO: This is almost identical to the prepare one, maybe we can make the method generic over it?
        let mut state = self.state.write();

        if let Some(pbft) = state.pbft_states.get(&level_update.tag.block_hash) {
            let aggregation = Arc::clone(&pbft.aggregation);
            let aggregation = aggregation.read();
            drop(state);
            aggregation.push_commit_level_update(level_update);
            let (prepare_votes, commit_votes) = aggregation.votes();
            debug!("pBFT: Prepare: {}, Commit: {}", fmt_vote_progress(prepare_votes), fmt_vote_progress(commit_votes));
        } else {
            warn!("Not in pBFT phase, but received prepare update: {:?}", level_update);
        }
    }

    // Public interface: start view changes, pBFT phase, push contributions

    /// Starts a new view-change
    pub fn start_view_change(&self, signed_view_change: SignedViewChange) -> Result<(), ValidatorNetworkError> {
        let view_change = signed_view_change.message.clone();
        let mut state = self.state.write();

        if let Some(aggregation) = state.view_changes.get(&view_change) {
            // Do nothing, but return an error. At some point the validator should increase the view number
            warn!("{:?} already exists with {} votes", signed_view_change.message, aggregation.votes());
            Err(ValidatorNetworkError::ViewChangeAlreadyExists(view_change))
        }
        else {
            let validators = self.blockchain.current_validators().clone();

            let node_id = state.validator_id.expect("Validator ID not set");
            assert_eq!(signed_view_change.signer_idx as usize, node_id);

            // Create view change aggregation
            let aggregation = ViewChangeAggregation::new(
                view_change.clone(),
                node_id,
                validators,
                Arc::clone(&state.active_validators),
                None
            );

            // Register handler for when done and start (or use Future)
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

            // Push our contribution
            aggregation.push_contribution(signed_view_change);

            state.view_changes.insert(view_change, aggregation);

            Ok(())
        }
    }

    /// Start pBFT phase with our proposal
    pub fn start_pbft(&self, signed_proposal: SignedPbftProposal) -> Result<(), ValidatorNetworkError> {
        //info!("Starting pBFT with proposal: {:?}", signed_proposal.message);
        self.on_pbft_proposal(signed_proposal)
    }

    pub fn push_prepare(&self, signed_prepare: SignedPbftPrepareMessage) -> Result<(), ValidatorNetworkError> {
        let state = self.state.read();
        if let Some(pbft) = state.pbft_states.get(&signed_prepare.message.block_hash) {
            let aggregation = Arc::clone(&pbft.aggregation);
            let aggregation = aggregation.read();
            drop(state);
            aggregation.push_signed_prepare(signed_prepare);
            Ok(())
        }
        else {
            Err(ValidatorNetworkError::NotInPbftPhase)
        }
    }

    pub fn push_commit(&self, signed_commit: SignedPbftCommitMessage) -> Result<(), ValidatorNetworkError> {
        let state = self.state.read();
        if let Some(pbft) = state.pbft_states.get(&signed_commit.message.block_hash) {
            let aggregation = Arc::clone(&pbft.aggregation);
            let aggregation = aggregation.read();
            drop(state);
            aggregation.push_signed_commit(signed_commit);
            Ok(())
        }
        else {
            Err(ValidatorNetworkError::NotInPbftPhase)
        }
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

/// Pretty-print voting progress
fn fmt_vote_progress(slots: usize) -> String {
    let done = slots >= (TWO_THIRD_SLOTS as usize);
    format!("votes={: >3} / {}, done={}", slots, SLOTS, done)
}
