use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::fmt;

use failure::Fail;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use tokio;
use futures::future;

use block_albatross::{
    BlockHeader,
    ForkProof, PbftProof, PbftProposal,
    PbftPrepareMessage, PbftCommitMessage,
    SignedPbftCommitMessage, SignedPbftPrepareMessage, SignedPbftProposal,
    SignedViewChange, ViewChange, ViewChangeProof
};
use block_albatross::signed::AggregateProof;
use blockchain_albatross::Blockchain;
use collections::grouped_list::Group;
use hash::{Blake2bHash, Hash};
use messages::{Message, ViewChangeProofMessage};
use network::{Network, NetworkEvent, Peer};
use network_primitives::validator_info::{SignedValidatorInfo};
use network_primitives::address::PeerId;
use primitives::policy::{SLOTS, TWO_THIRD_SLOTS, is_macro_block_at};
use primitives::validators::IndexedSlot;
use utils::mutable_once::MutableOnce;
use utils::observer::{PassThroughNotifier, weak_listener, weak_passthru_listener};
use handel::aggregation::AggregationEvent;
use handel::update::LevelUpdateMessage;

use crate::validator_agent::{ValidatorAgent, ValidatorAgentEvent};
use crate::signature_aggregation::view_change::ViewChangeAggregation;
use crate::signature_aggregation::pbft::PbftAggregation;
use crate::pool::ValidatorPool;


#[derive(Clone, Debug, Fail)]
pub enum ValidatorNetworkError {
    #[fail(display = "View change already in progress: {:?}", _0)]
    ViewChangeAlreadyExists(ViewChange),

    #[fail(display = "Already got another pBFT proposal at this view")]
    ProposalCollision,
    #[fail(display = "Unknown pBFT proposal")]
    UnknownProposal,
    #[fail(display = "Invalid pBFT proposal")]
    InvalidProposal,
}

#[derive(Clone, Debug)]
pub enum ValidatorNetworkEvent {
    /// When a fork proof was given
    ForkProof(Box<ForkProof>),

    /// When a valid view change was completed
    ViewChangeComplete(Box<(ViewChange, ViewChangeProof)>),

    /// When a valid macro block is proposed by the correct pBFT-leader. This can happen multiple
    /// times during an epoch - i.e. when a proposal with a higher view number is received.
    PbftProposal(Box<(Blake2bHash, PbftProposal)>),

    /// When enough prepare signatures are collected for a proposed macro block
    PbftPrepareComplete(Box<(Blake2bHash, PbftProposal)>),

    /// When the pBFT proof is complete
    PbftComplete(Box<(Blake2bHash, PbftProposal, PbftProof)>),
}


/// State of current pBFT phase
#[derive(Clone)]
struct PbftState {
    /// The proposed macro block with justification
    proposal: SignedPbftProposal,

    /// The hash of the header of the proposed macro block
    block_hash: Blake2bHash,

    /// The state of the signature aggregation for pBFT prepare and commit
    aggregation: Arc<RwLock<PbftAggregation>>,
}

impl PbftState {
    pub fn new(block_hash: Blake2bHash, proposal: SignedPbftProposal, node_id: usize, validators: Arc<RwLock<ValidatorPool>>) -> Self {
        let aggregation = Arc::new(RwLock::new(PbftAggregation::new(block_hash.clone(), node_id, validators, None)));
        Self {
            proposal,
            block_hash,
            aggregation,
        }
    }

    // Only works on non-buffered proposals
    fn check_verified(&self, chain: &Blockchain) -> bool {
        let block_number = self.proposal.message.header.block_number;
        let view_number = self.proposal.message.header.view_number;

        // Can we verify validity of macro block?
        let IndexedSlot { slot, .. } = chain.get_block_producer_at(block_number, view_number, None)
            .expect("check_verified() called without enough micro blocks");

        // Check the signer index
        if let Some(ref validator) = chain.get_current_validator_by_idx(self.proposal.signer_idx) {
            let Group(_, validator_key) = validator;
            // Does the key own the current slot?
            if validator_key != &slot.public_key {
                return false;
            }
        } else {
            // No validator at this index
            return false;
        }

        // Check the validity of the block
        // TODO: We check the view change proof the second time here if previously buffered
        if let Err(e) = chain.verify_block_header(
            &BlockHeader::Macro(self.proposal.message.header.clone()),
            self.proposal.message.view_change.as_ref().into(),
            &slot.public_key.uncompress().unwrap(),
            None // TODO Would it make sense to pass a Read transaction?
        ) {
            debug!("[PBFT-PROPOSAL] Invalid macro block header: {:?}", e);
            return false;
        }

        // Check the signature of the proposal
        let public_key = &slot.public_key.uncompress_unchecked();
        if !self.proposal.verify(&public_key) {
            debug!("[PBFT-PROPOSAL] Invalid signature");
            return false;
        }

        true
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

    /// Maps (view-change-number, block-number) to the proof that is being aggregated
    /// and a flag whether it's finalized. clear after macro block
    view_changes: HashMap<ViewChange, ViewChangeAggregation>,
    complete_view_changes: HashMap<ViewChange, ViewChangeProof>,

    /// If we're in pBFT phase, this is the current state of it
    pbft_states: Vec<PbftState>,

    /// If we're an active validator, set our validator ID here
    validator_id: Option<usize>,
}

impl ValidatorNetworkState {
    pub(crate) fn get_pbft_state(&self, hash: &Blake2bHash) -> Option<&PbftState> {
        self.pbft_states.iter().find(|state| &state.block_hash == hash)
    }

    pub(crate) fn get_pbft_state_mut(&mut self, hash: &Blake2bHash) -> Option<&mut PbftState> {
        self.pbft_states.iter_mut().find(|state| &state.block_hash == hash)
    }
}

pub struct ValidatorNetwork {
    blockchain: Arc<Blockchain<'static>>,

    /// The signed validator info for this node
    info: SignedValidatorInfo,

    /// The validator network state
    state: RwLock<ValidatorNetworkState>,

    /// Stores validator contact information and holds references to connected validators
    validators: Arc<RwLock<ValidatorPool>>,

    self_weak: MutableOnce<Weak<ValidatorNetwork>>,
    pub notifier: RwLock<PassThroughNotifier<'static, ValidatorNetworkEvent>>,
}

impl ValidatorNetwork {
    const MAX_VALIDATOR_INFOS: usize = 64;

    pub fn new(network: Arc<Network<Blockchain<'static>>>, blockchain: Arc<Blockchain<'static>>, info: SignedValidatorInfo) -> Arc<Self> {
        let mut pool = ValidatorPool::new(Arc::clone(&network));

        // blacklist ourself
        pool.blacklist(info.message.public_key.clone());

        let this = Arc::new(ValidatorNetwork {
            blockchain,
            info,
            state: RwLock::new(ValidatorNetworkState::default()),
            validators: Arc::new(RwLock::new(pool)),
            self_weak: MutableOnce::new(Weak::new()),
            notifier: RwLock::new(PassThroughNotifier::new()),
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
                    ValidatorAgentEvent::ValidatorInfos(infos) => {
                        this.on_validator_infos(infos);
                    },
                    ValidatorAgentEvent::ForkProof(fork_proof) => {
                        this.on_fork_proof(*fork_proof);
                    }
                    ValidatorAgentEvent::ViewChange(update_message) => {
                        this.on_view_change_level_update(*update_message);
                    },
                    ValidatorAgentEvent::ViewChangeProof(view_change_proof) => {
                        let ViewChangeProofMessage { view_change, proof } = *view_change_proof;
                        tokio::spawn(future::lazy(move || {
                            this.on_view_change_proof(view_change, proof);
                            Ok(())
                        }));
                    },
                    ValidatorAgentEvent::PbftProposal(proposal) => {
                        this.on_pbft_proposal(*proposal)
                            .unwrap_or_else(|e| debug!("Rejecting pBFT proposal: {}", e));
                    },
                    ValidatorAgentEvent::PbftPrepare(level_update) => {
                        this.on_pbft_prepare_level_update(*level_update);
                    },
                    ValidatorAgentEvent::PbftCommit(level_update) => {
                        this.on_pbft_commit_level_update(*level_update);
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
            self.validators.write().on_validator_left(agent);
        }
    }

    /// NOTE: assumes that the signature of the validator info was checked by the `ValidatorAgent`
    fn on_validator_infos(&self, infos: Vec<SignedValidatorInfo>) {
        let mut relay = Vec::new();

        for info in infos {
            trace!("Validator info: {:?}", info.message);
            let agent = self.state.read().agents.get(&info.message.peer_address.peer_id).cloned();
            let is_new = self.validators.write().on_validator_info(&info, agent);
            if is_new {
                relay.push(info);
            }
        }

        // relay
        if !relay.is_empty() {
            self.broadcast_potential(Message::ValidatorInfo(relay));
        }
    }

    fn on_fork_proof(&self, fork_proof: ForkProof) {
        self.notifier.read().notify(ValidatorNetworkEvent::ForkProof(Box::new(fork_proof.clone())));
        self.broadcast_fork_proof(fork_proof);
    }

    /// Called when we reach finality - i.e. when a macro block was produced. This must be called be the
    /// validator.
    ///
    /// `validator_id`: The index of the validator (a.k.a `pk_idx`), if we're active
    pub fn reset_epoch(&self, validator_id: Option<usize>) {
        trace!("Clearing view change and pBFT proof");
        let mut state = self.state.write();

        // Clear view changes
        state.view_changes.clear();

        // Clear pBFT states
        state.pbft_states.clear();

        // Set validator ID
        state.validator_id = validator_id;

        // Create mapping from validator ID to agent/peer
        // reset validator pool for new epoch
        self.validators.write().reset_epoch(&self.blockchain.current_validators());
    }

    /// Called when a new block is added
    pub fn on_blockchain_changed(&self, _hash: &Blake2bHash) {
        let mut state = self.state.write();
        let new_height = self.blockchain.block_number();

        let cancel_view_changes = state.view_changes.keys()
            .filter(|view_change| view_change.block_number <= new_height)
            .cloned()
            .collect::<Vec<ViewChange>>();
        for view_change in &cancel_view_changes {
            debug!("Deleting view change {}", view_change);
            state.view_changes.remove(view_change);
        }

        // The rest of this function switches the state from buffered to complete.
        // Only the proposal with the highest view number will remain.

        // Check if next block will be a macro block
        if !is_macro_block_at(new_height + 1) {
            return;
        }

        // Remove invalid proposals
        state.pbft_states.retain(|pbft| {
            let verified = pbft.check_verified(&self.blockchain);
            if !verified {
                debug!("Buffered pBFT proposal confirmed invalid: {}", &pbft.block_hash);
            } else {
                debug!("Verified pBFT proposal: {}", pbft.block_hash);
            }
            verified
        });

        if state.pbft_states.is_empty() {
            return;
        }

        // Remove proposals that collide on the same views
        state.pbft_states.dedup_by_key(|pbft| {
            let header = &pbft.proposal.message.header;
            (header.block_number, header.view_number)
        });

        // Choose proposal with the highest view number
        let best_pbft = state.pbft_states.iter()
            .max_by_key(|pbft| pbft.proposal.message.header.view_number)
            .unwrap().clone();
        state.pbft_states = vec![best_pbft.clone()];

        // We need to drop the state before notifying and relaying
        drop(state);

        // Notify Validator (and send prepare message)
        let block_hash = best_pbft.proposal.message.header.hash::<Blake2bHash>();
        self.notifier.read().notify(ValidatorNetworkEvent::PbftProposal(Box::new((block_hash, best_pbft.proposal.message))));
    }

    /// Pushes the update to the signature aggregation for this view-change
    fn on_view_change_level_update(&self, update_message: LevelUpdateMessage<ViewChange>) {
        let state = self.state.upgradable_read();

        // check if we already completed this view change
        if state.complete_view_changes.contains_key(&update_message.tag) {
            debug!("View change already complete: {}", update_message.tag);
            return;
        }

        if let Some(aggregation) = state.view_changes.get(&update_message.tag) {
            aggregation.push_update(update_message);
            debug!("View change: {}", fmt_vote_progress(aggregation.votes()));
        }
        else if let Some(node_id) = state.validator_id {
            let view_change = update_message.tag.clone();

            // create view change
            let aggregation = self.new_view_change(view_change.clone(), node_id);

            // add update
            aggregation.push_update(update_message);

            let mut state = RwLockUpgradableReadGuard::upgrade(state);
            state.view_changes.insert(view_change, aggregation);
        }
    }

    /// When we receive a complete view change proof
    fn on_view_change_proof(&self, view_change: ViewChange, proof: ViewChangeProof) {
        // pub fn verify(&self, message: &M, validators: &GroupedList<LazyPublicKey>, threshold: u16) -> Result<(), AggregateProofError>
        if let Err(e) = proof.verify(&view_change, &self.blockchain.current_validators(), TWO_THIRD_SLOTS) {
            // TODO: Make this use Display instead, once the implementation for it is merged.
            debug!("Invalid view change proof: {:?}", e);
        }

        let state = self.state.upgradable_read();
        if state.complete_view_changes.contains_key(&view_change) {
            return;
        }

        info!("Received view change proof for: {}", view_change);
        let mut state = RwLockUpgradableReadGuard::upgrade(state);

        // put into complete view changes
        state.complete_view_changes.insert(view_change.clone(), proof.clone());

        // remove active view change
        state.view_changes.remove(&view_change);

        // remove all active view changes that are now obsolete
        // this was meant to also kill old view changes from older blocks even, do we need this?
        /*let cancel_view_changes = state.view_changes.keys()
            .filter(|vc| vc.new_view_number <= view_change.new_view_number)
            .cloned()
            .collect::<Vec<ViewChange>>();
        for view_change in cancel_view_changes {
            state.view_changes.remove(&view_change)
                .unwrap_or_else(|| panic!("Expected active view change {:?}", view_change));
        }*/

        drop(state);

        // notify validator
        self.notifier.read()
            .notify(ValidatorNetworkEvent::ViewChangeComplete(Box::new((view_change.clone(), proof.clone()))));

        // broadcast
        self.broadcast_view_change_proof(view_change, proof);

    }

    /// Start pBFT with the given proposal.
    /// Either we generated that proposal, or we received it
    /// Proposal yet to be verified
    pub fn on_pbft_proposal(&self, signed_proposal: SignedPbftProposal) -> Result<(), ValidatorNetworkError> {
        let mut state = self.state.write();
        let block_hash = signed_proposal.message.header.hash::<Blake2bHash>();

        if state.get_pbft_state(&block_hash).is_some() {
            // Proposal already known, ignore
            trace!("Ignoring known pBFT proposal: {}", &block_hash);
            return Ok(());
        }

        debug!("pBFT proposal by validator {}: {}", signed_proposal.signer_idx, block_hash);

        let validator_id = match state.validator_id {
            Some(validator_id) => validator_id,
            None => {
                debug!("Received proposal, but validator is not active");
                return Ok(())
            },
        };

        let pbft = PbftState::new(
            block_hash.clone(),
            signed_proposal.clone(),
            validator_id,
            Arc::clone(&self.validators),
        );

        let chain_height = self.blockchain.height();
        let buffered = !is_macro_block_at(chain_height + 1);

        // Check validity if proposal not buffered
        if !buffered {
            let verified = pbft.check_verified(&self.blockchain);
            if !verified {
                return Err(ValidatorNetworkError::InvalidProposal);
            }

            // Check if another proposal has same or greater view number
            let header = &pbft.proposal.message.header;
            let other_header = state.pbft_states.iter()
                .map(|pbft| &pbft.proposal.message.header)
                .find(|other_header| header.view_number <= other_header.view_number);
            if let Some(other_header) = other_header {
                if header.view_number == other_header.view_number {
                    return Err(ValidatorNetworkError::ProposalCollision);
                } else {
                    return Ok(());
                }
            }
        }

        // The prepare handler. This will store the finished prepare proof in the pBFT state
        let key = block_hash.clone();
        pbft.aggregation.read().prepare_aggregation.notifier.write()
            .register(weak_passthru_listener(Weak::clone(&self.self_weak), move |this, event| {
                match event {
                    AggregationEvent::Complete { best } => {
                        let event = if let Some(pbft) = this.state.write().get_pbft_state_mut(&key) {
                            trace!("Prepare complete. Signers: {}", best.signers);

                            // Return the event
                            Some(ValidatorNetworkEvent::PbftPrepareComplete(Box::new((pbft.block_hash.clone(), pbft.proposal.message.clone()))))
                        } else {
                            error!("No pBFT state");
                            None
                        };
                        // If we generated a prepare complete event, notify the validator
                        if let Some(event) = event {
                            this.notifier.read().notify(event)
                        }
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
                        let event = if let Some(pbft) = this.state.write().get_pbft_state_mut(&key) {
                            // Build commit proof
                            let commit_proof = AggregateProof::new(best.signature, best.signers);
                            trace!("Commit complete: {:?}", commit_proof);

                            // NOTE: The commit evaluator will only mark the signature as final, if there are enough commit signatures from validators that also
                            //       signed prepare messages. Thus a complete prepare proof must exist at this point.
                            // Take the best prepare signature we have to this point
                            let prepare_signature = pbft.aggregation.read().prepare_signature().expect("Prepare signature missing");
                            let prepare_proof = AggregateProof::new(prepare_signature.signature, prepare_signature.signers);

                            let pbft_proof = PbftProof { prepare: prepare_proof, commit: commit_proof };

                            // Return the event
                            Some(ValidatorNetworkEvent::PbftComplete(Box::new((pbft.block_hash.clone(), pbft.proposal.message.clone(), pbft_proof))))
                        } else {
                            error!("No pBFT state");
                            None
                        };
                        // If we generated a prepare complete event, notify the validator
                        if let Some(event) = event {
                            this.notifier.read().notify(event)
                        }
                    }
                }
            }));

        if !buffered {
            // Replace pBFT state
            state.pbft_states = vec![pbft];
        } else {
            // Add pBFT state
            state.pbft_states.push(pbft);
        }

        // We need to drop the state before notifying and relaying
        drop(state);

        // Notify Validator (and send prepare message)
        if !buffered {
            self.notifier.read().notify(ValidatorNetworkEvent::PbftProposal(Box::new((block_hash.clone(), signed_proposal.message.clone()))));
        }

        // Broadcast to other validators
        self.broadcast_pbft_proposal(signed_proposal);

        Ok(())
    }

    pub fn on_pbft_prepare_level_update(&self, level_update: LevelUpdateMessage<PbftPrepareMessage>) {
        let state = self.state.read();

        trace!("Prepare level update: {:#?}", level_update);

        if let Some(pbft) = state.get_pbft_state(&level_update.tag.block_hash) {
            let aggregation = Arc::clone(&pbft.aggregation);
            let aggregation = aggregation.read();
            drop(state);
            aggregation.push_prepare_level_update(level_update);
            let (prepare_votes, commit_votes) = aggregation.votes();
            debug!("pBFT: Prepare: {}, Commit: {}", fmt_vote_progress(prepare_votes), fmt_vote_progress(commit_votes));
        }
    }

    pub fn on_pbft_commit_level_update(&self, level_update: LevelUpdateMessage<PbftCommitMessage>) {
        // TODO: This is almost identical to the prepare one, maybe we can make the method generic over it?
        let state = self.state.read();

        trace!("Commit level update: {:#?}", level_update);

        if let Some(pbft) = state.get_pbft_state(&level_update.tag.block_hash) {
            let aggregation = Arc::clone(&pbft.aggregation);
            let aggregation = aggregation.read();
            drop(state);
            aggregation.push_commit_level_update(level_update);
            let (prepare_votes, commit_votes) = aggregation.votes();
            debug!("pBFT: Prepare: {}, Commit: {}", fmt_vote_progress(prepare_votes), fmt_vote_progress(commit_votes));
        }
    }

    // Public interface: start view changes, pBFT phase, push contributions

    /// Starts a new view-change
    pub fn start_view_change(&self, signed_view_change: SignedViewChange) {
        let view_change = signed_view_change.message.clone();
        let mut state = self.state.write();

        if let Some(aggregation) = state.view_changes.get(&view_change) {
            aggregation.push_contribution(signed_view_change);
        }
        else {
            let node_id = state.validator_id.expect("Validator ID not set");
            assert_eq!(signed_view_change.signer_idx as usize, node_id);

            let aggregation = self.new_view_change(view_change.clone(), node_id);
            aggregation.push_contribution(signed_view_change);
            state.view_changes.insert(view_change, aggregation);
        }
    }

    fn new_view_change(&self, view_change: ViewChange, node_id: usize) -> ViewChangeAggregation {
        // Create view change aggregation
        let aggregation = ViewChangeAggregation::new(
            view_change.clone(),
            node_id,
            Arc::clone(&self.validators),
            None
        );
        debug!("New view change for: {}, node_id={}", view_change, node_id);

        // Register handler for when done and start (or use Future)
        aggregation.inner.notifier.write().register(weak_passthru_listener(Weak::clone(&self.self_weak), move |this, event| {
            match event {
                AggregationEvent::Complete { best } => {
                    let view_change = view_change.clone();
                    tokio::spawn(future::lazy(move || {
                        let proof = ViewChangeProof::new(best.signature, best.signers);
                        this.on_view_change_proof(view_change, proof);
                        Ok(())
                    }));
                }
            }
        }));

        aggregation
    }

    /// Start pBFT phase with our proposal
    pub fn start_pbft(&self, signed_proposal: SignedPbftProposal) -> Result<(), ValidatorNetworkError> {
        //info!("Starting pBFT with proposal: {:?}", signed_proposal.message);
        self.on_pbft_proposal(signed_proposal)
    }

    pub fn push_prepare(&self, signed_prepare: SignedPbftPrepareMessage) -> Result<(), ValidatorNetworkError> {
        trace!("Push prepare: {:#?}", signed_prepare);
        let state = self.state.read();
        if let Some(pbft) = state.get_pbft_state(&signed_prepare.message.block_hash) {
            let aggregation = Arc::clone(&pbft.aggregation);
            let aggregation = aggregation.read();
            drop(state);
            aggregation.push_signed_prepare(signed_prepare);
            Ok(())
        }
        else {
            Err(ValidatorNetworkError::UnknownProposal)
        }
    }

    pub fn push_commit(&self, signed_commit: SignedPbftCommitMessage) -> Result<(), ValidatorNetworkError> {
        trace!("Push commit: {:#?}", signed_commit);
        let state = self.state.read();
        if let Some(pbft) = state.get_pbft_state(&signed_commit.message.block_hash) {
            let aggregation = Arc::clone(&pbft.aggregation);
            let aggregation = aggregation.read();
            drop(state);
            aggregation.push_signed_commit(signed_commit);
            Ok(())
        }
        else {
            Err(ValidatorNetworkError::UnknownProposal)
        }
    }

    // Legacy broadcast methods -------------------------
    //
    // These are still used to relay `ValidatorInfo` and `PbftProposal`

    /// Broadcast to all known validators
    fn broadcast_potential(&self, msg: Message) {
        trace!("Broadcast to potential validators: {}", msg.ty());
        for agent in self.validators.read().iter_potential() {
            agent.peer.channel.send_or_close(msg.clone());
        }
    }

    /// Broadcast to all known validators
    fn broadcast_active(&self, msg: Message) {
        trace!("Broadcast to active validators: {}", msg.ty());
        for agent in self.validators.read().iter_active() {
            agent.peer.channel.send_or_close(msg.clone());
        }
    }

    /// Broadcast pBFT proposal
    fn broadcast_pbft_proposal(&self, proposal: SignedPbftProposal) {
        self.broadcast_active(Message::PbftProposal(Box::new(proposal)));
    }

    /// Broadcast fork-proof
    fn broadcast_fork_proof(&self, fork_proof: ForkProof) {
        self.broadcast_active(Message::ForkProof(Box::new(fork_proof)));
    }

    fn broadcast_view_change_proof(&self, view_change: ViewChange, proof: ViewChangeProof) {
        self.broadcast_active(Message::ViewChangeProof(Box::new(ViewChangeProofMessage {
            view_change, proof
        })));
    }
}

/// Pretty-print voting progress
fn fmt_vote_progress(slots: usize) -> String {
    let done = slots >= (TWO_THIRD_SLOTS as usize);
    format!("votes={: >3} / {}, done={}", slots, SLOTS, done)
}
