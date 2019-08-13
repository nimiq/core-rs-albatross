use std::sync::Arc;
use std::cmp::Ordering;
use std::collections::binary_heap::BinaryHeap;
use std::fmt;

use network_primitives::validator_info::{ValidatorInfo, SignedValidatorInfo};
use network_primitives::address::PeerId;
use network::Peer;
use utils::observer::{PassThroughNotifier, weak_passthru_listener, weak_listener};
use parking_lot::RwLock;
use bls::bls12_381::{PublicKey, CompressedPublicKey};
use collections::grouped_list::Group;
use block_albatross::{SignedViewChange, SignedPbftPrepareMessage, SignedPbftCommitMessage,
                      SignedPbftProposal, ViewChange, ForkProof, BlockHeader, PbftPrepareMessage,
                      PbftCommitMessage};
use primitives::policy;
use primitives::validators::IndexedSlot;
use blockchain_albatross::Blockchain;
use blockchain_base::BlockchainEvent;
use handel::update::LevelUpdateMessage;


pub enum ValidatorAgentEvent {
    ValidatorInfo(SignedValidatorInfo),
    ForkProof(ForkProof),
    ViewChange(LevelUpdateMessage<ViewChange>),
    PbftProposal(SignedPbftProposal),
    PbftPrepare(LevelUpdateMessage<PbftPrepareMessage>),
    PbftCommit(LevelUpdateMessage<PbftCommitMessage>),
}

/// Small wrapper to sort buffered pBFT proposals by block number
#[derive(Clone, Debug)]
struct BufferedPbftProposal(SignedPbftProposal);
impl Eq for BufferedPbftProposal {}
impl Ord for BufferedPbftProposal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.message.header.block_number
            .cmp(&other.0.message.header.block_number)
            .then(self.0.message.header.view_number
                      .cmp(&other.0.message.header.view_number))
    }
}
impl PartialOrd for BufferedPbftProposal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}
impl PartialEq for BufferedPbftProposal {
    fn eq(&self, other: &Self) -> bool {
        self.0.message.header.block_number == other.0.message.header.block_number
            && self.0.message.header.view_number == other.0.message.header.view_number
    }
}

pub struct ValidatorAgentState {
    pub(crate) validator_info: Option<SignedValidatorInfo>,
    proposal_buf: BinaryHeap<BufferedPbftProposal>,
}

pub struct ValidatorAgent {
    pub(crate) peer: Arc<Peer>,
    pub(crate) blockchain: Arc<Blockchain<'static>>,
    pub(crate) state: RwLock<ValidatorAgentState>,
    pub notifier: RwLock<PassThroughNotifier<'static, ValidatorAgentEvent>>,
}

impl ValidatorAgent {
    pub fn new(peer: Arc<Peer>, blockchain: Arc<Blockchain<'static>>) -> Arc<Self> {
        let agent = Arc::new(Self {
            peer,
            blockchain,
            state: RwLock::new(ValidatorAgentState {
                validator_info: None,
                proposal_buf: BinaryHeap::new(),
            }),
            notifier: RwLock::new(PassThroughNotifier::new()),
        });

        Self::init_listeners(&agent);

        agent
    }

    fn init_listeners(this: &Arc<Self>) {
        this.peer.channel.msg_notifier.validator_info.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, signed_infos: Vec<SignedValidatorInfo>| {
                this.on_validator_infos(signed_infos);
            }));
        this.peer.channel.msg_notifier.fork_proof.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, fork_proof| {
                this.on_fork_proof_message(fork_proof);
            }));

        this.peer.channel.msg_notifier.pbft_proposal.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, proposal| {
                trace!("Received macro block proposal: {:?}", &proposal);
                this.on_pbft_proposal_message(proposal);
            }));

        this.peer.channel.msg_notifier.pbft_prepare.write()
            .register(weak_passthru_listener(Arc::downgrade(this),
                |this, prepare| this.on_pbft_prepare_message(prepare)));
        this.peer.channel.msg_notifier.pbft_commit.write()
            .register(weak_passthru_listener(Arc::downgrade(this),
                |this, commit| this.on_pbft_commit_message(commit)));
        this.peer.channel.msg_notifier.view_change.write()
            .register(weak_passthru_listener( Arc::downgrade(this), |this, signed_view_change| {
                this.on_view_change_message(signed_view_change);
            }));

        this.blockchain.notifier.write()
            .register(weak_listener(Arc::downgrade(this), |this, event| {
                if let BlockchainEvent::Extended(_) = event {
                    let block_number = this.blockchain.block_number();
                    if policy::is_macro_block_at(block_number + 1) {
                        // Do this asynchronously, the handler shouldn't do this work
                        tokio::spawn(futures::future::lazy(move || {
                            let mut proposals: Vec<SignedPbftProposal> = Vec::new();
                            let mut state = this.state.write();
                            let view_number = this.blockchain.view_number();

                            trace!("Blockchain extended, replaying proposals");
                            trace!("Blockchain: number=#{}.{}", block_number, view_number);

                            // get all proposals that are now valid
                            while let Some(proposal) = state.proposal_buf.peek() {
                                if proposal.0.message.header.block_number <= block_number + 1 {
                                    // unwrap is safe, since we already peeked and have the write lock
                                    proposals.push(state.proposal_buf.pop().unwrap().0);
                                } else {
                                    break;
                                }
                            }

                            // drop lock
                            // NOTE: Since we basically pretend to just receive the proposals now, it
                            //       doesn't matter that we have concurrency here
                            drop(state);

                            // replay proposals
                            for proposal in proposals {
                                this.on_pbft_proposal_message(proposal);
                            }

                            Ok(())
                        }));
                    }
                }
            }));
    }

    /// When a list of validator infos is received, verify the signatures and notify
    fn on_validator_infos(&self, signed_infos: Vec<SignedValidatorInfo>) {
        debug!("[VALIDATOR-INFO] contains {} validator infos", signed_infos.len());
        for signed_info in signed_infos {
            // TODO: first check if we already know this validator. If so, we don't need to check
            // the signature of this info.
            if let Ok(public_key) = signed_info.message.public_key.uncompress() {
                let signature_okay = signed_info.verify(&public_key);
                trace!("[VALIDATOR-INFO] {:#?}, signature_okay={}", signed_info.message, signature_okay);
                if !signature_okay {
                    continue;
                }
                self.notifier.read().notify(ValidatorAgentEvent::ValidatorInfo(signed_info));
            }
            else {
                error!("Uncompressing public key failed: {}", signed_info.message.peer_address);
            }
        }
    }

    /// When a fork proof message is received
    fn on_fork_proof_message(&self, fork_proof: ForkProof) {
        debug!("[FORK-PROOF] Fork proof:");

        if !fork_proof.is_valid_at(self.blockchain.block_number() + 1) {
            debug!("[FORK-PROOF] Not valid");
            return;
        }

        debug!("[FORK-PROOF] Header 1: {:?}", fork_proof.header1);
        debug!("[FORK_PROOF] Header 2: {:?}", fork_proof.header2);
        let block_number = fork_proof.block_number();
        let view_number = fork_proof.view_number();

        let producer = self.blockchain.get_block_producer_at(block_number, view_number, None);
        if producer.is_none() {
            debug!("[FORK-PROOF] Unknown block producer for #{}.{}", block_number, view_number);
            return;
        }

        let slot = producer.unwrap().slot;
        if let Err(e) = fork_proof.verify(&slot.public_key.uncompress_unchecked()) {
            debug!("[FORK-PROOF] Invalid signature in fork proof: {:?}", e);
            return;
        }

        self.notifier.read().notify(ValidatorAgentEvent::ForkProof(fork_proof));
    }

    /// When a view change message is received, verify the signature and pass it to ValidatorNetwork
    fn on_view_change_message(&self, update_message: LevelUpdateMessage<ViewChange>) {
        trace!("[VIEW-CHANGE] Received: number=#{}.{} update={:?} peer={}",
               update_message.tag.block_number,
               update_message.tag.new_view_number,
               update_message.update,
               self.peer.peer_address());

        let blockchain_epoch = policy::epoch_at(self.blockchain.block_number());
        let view_change_epoch = policy::epoch_at(update_message.tag.block_number);
        match view_change_epoch.cmp(&blockchain_epoch) {
            Ordering::Greater => {
                debug!("[VIEW-CHANGE] Ignoring view change message for a future epoch: epoch={} block_number=#{}",
                       view_change_epoch,
                       update_message.tag.block_number);
                return;
            },
            Ordering::Less => {
                debug!("[VIEW-CHANGE] Ignoring view change message for an old epoch: epoch={} block_number=#{}",
                       view_change_epoch,
                       update_message.tag.block_number);
                return;
            },
            Ordering::Equal => (),
        };

        self.notifier.read().notify(ValidatorAgentEvent::ViewChange(update_message))
    }

    /// When a pbft block proposal is received
    fn on_pbft_proposal_message(&self, proposal: SignedPbftProposal) {
        let block_number = proposal.message.header.block_number;
        let view_number = proposal.message.header.view_number;

        // Reject proposal if block number already known
        if proposal.message.header.block_number <= self.blockchain.head().block_number() {
            trace!("[PBFT-PROPOSAL] Ignoring old proposal: {:?}", proposal.message.header);
            return;
        }

        if let Some(IndexedSlot { slot, .. }) = self.blockchain.get_block_producer_at(block_number, view_number, None) {
            let public_key = slot.public_key.uncompress_unchecked();

            // check the validity of the block
            if let Err(e) = self.blockchain.verify_block_header(&BlockHeader::Macro(proposal.message.header.clone()), proposal.message.view_change.as_ref().into(), &public_key, None) {
                debug!("[PBFT-PROPOSAL] Invalid macro block header: {:?}", e);
                return;
            }

            // check the signature of the proposal
            // XXX We ignore the `pk_idx` field in the `SignedMessage`
            if !proposal.verify(&public_key) {
                debug!("[PBFT-PROPOSAL] Invalid signature");
                return;
            }

            // check the view change proof
            if let Some(view_change_proof) = &proposal.message.view_change {
                let view_change = ViewChange { block_number, new_view_number: view_number };
                if view_change_proof.verify(&view_change, &self.blockchain.current_validators(), policy::TWO_THIRD_SLOTS).is_err() {
                    debug!("[PBFT-PROPOSAL] Invalid view change proof: {:?}", view_change_proof);
                    return;
                }
            }


            self.notifier.read().notify(ValidatorAgentEvent::PbftProposal(proposal));
        }
        else {
            // TODO: Limit this somehow
            if proposal.message.header.block_number > self.blockchain.block_number() + 1 {
                debug!("[PBFT-PROPOSAL] Missing micro blocks for proposal.");
                self.state.write().proposal_buf.push(BufferedPbftProposal(proposal))
            }
            else {
                warn!("[PBFT-PROPOSAL] No pBFT leader");
            }
        }
    }

    /// When a pbft prepare message is received, verify the signature and pass it to ValidatorNetwork
    /// TODO: The validator network could just register this it-self
    fn on_pbft_prepare_message(&self, level_update: LevelUpdateMessage<PbftPrepareMessage>) {
        trace!("[PBFT-PREPARE] Received: block_hash={} update={:?} peer={}",
               level_update.tag.block_hash,
               level_update.update,
               self.peer.peer_address());

        self.notifier.read().notify(ValidatorAgentEvent::PbftPrepare(level_update));
    }

    /// When a pbft commit message is received, verify the signature and pass it to ValidatorNetwork
    /// FIXME This will verify a commit message with the current validator set, not with the one for
    /// which this commit is for.
    fn on_pbft_commit_message(&self, level_update: LevelUpdateMessage<PbftCommitMessage>) {
        trace!("[PBFT-COMMIT] Received: block_hash={} update={:?} peer={}",
               level_update.tag.block_hash,
               level_update.update,
               self.peer.peer_address());

        self.notifier.read().notify(ValidatorAgentEvent::PbftCommit(level_update));
    }

    pub fn validator_info(&self) -> Option<ValidatorInfo> {
        self.state.read().validator_info.as_ref().map(|signed| signed.message.clone())
    }

    pub fn public_key(&self) -> Option<CompressedPublicKey> {
        self.state.read().validator_info.as_ref().map(|info| info.message.public_key.clone())
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer.peer_address().peer_id.clone()
    }
}

impl fmt::Debug for ValidatorAgent {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "ValidatorAgent {{ peer_id: {}, public_key: {:?} }}", self.peer_id(), self.public_key())
    }
}