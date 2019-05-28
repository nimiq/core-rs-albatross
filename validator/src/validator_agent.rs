use std::sync::Arc;
use network_primitives::validator_info::{SignedValidatorInfo, ValidatorId};
use network::Peer;
use utils::observer::{PassThroughNotifier, weak_passthru_listener};
use parking_lot::RwLock;
use bls::bls12_381::{CompressedPublicKey, PublicKey};
use block_albatross::{SignedViewChange, SignedPbftPrepareMessage, SignedPbftCommitMessage,
                      SignedPbftProposal, ViewChange, Block, MacroBlock, ForkProof};
use primitives::policy::TWO_THIRD_VALIDATORS;
use blockchain_albatross::Blockchain;
use hash::{Hash, Blake2bHash};


pub enum ValidatorAgentEvent {
    ValidatorInfo(SignedValidatorInfo),
    ForkProof(ForkProof),
    ViewChange { view_change: SignedViewChange, public_key: PublicKey, slots: u16 },
    PbftProposal(SignedPbftProposal),
    PbftPrepare { prepare: SignedPbftPrepareMessage, public_key: PublicKey, slots: u16 },
    PbftCommit { commit: SignedPbftCommitMessage, public_key: PublicKey, slots: u16 }
}

pub struct ValidatorAgentState {
    pub(crate) validator_info: Option<SignedValidatorInfo>,
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
        this.peer.channel.msg_notifier.view_change.write()
            .register(weak_passthru_listener( Arc::downgrade(this), |this, signed_view_change| {
                trace!("Received view change message: {:?}", &signed_view_change);
                this.on_view_change_message(signed_view_change);
            }));
        this.peer.channel.msg_notifier.pbft_proposal.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, proposal| {
                trace!("Received macro block proposal: {:?}", &proposal);
                this.on_pbft_proposal_message(proposal);
            }));
        this.peer.channel.msg_notifier.pbft_prepare.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, prepare| {
                trace!("Received pBFT prepare: {:?}", &prepare);
                this.on_pbft_prepare_message(prepare);
            }));
        this.peer.channel.msg_notifier.pbft_commit.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, commit| {
                trace!("Received pBFT commit: {:?}", &commit);
                this.on_pbft_commit_message(commit);
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
                debug!("[VALIDATOR-INFO] {:#?}, signature_okay={}", signed_info.message, signature_okay);
                if !signature_okay {
                    continue;
                }
                self.notifier.read().notify(ValidatorAgentEvent::ValidatorInfo(signed_info));
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

        let producer = self.blockchain.get_block_producer_at(block_number, view_number);
        if producer.is_none() {
            debug!("[FORK-PROOF] Unknown block producer: block_number={}, view_number={}", block_number, view_number);
            return;
        }

        let slot = producer.unwrap().1;
        if let Err(e) = fork_proof.verify(&slot.public_key.uncompress_unchecked()) {
            debug!("[FORK-PROOF] Invalid signature in fork proof: {:?}", e);
            return;
        }

        self.notifier.read().notify(ValidatorAgentEvent::ForkProof(fork_proof));
    }

    /// When a view change message is received, verify the signature and pass it to ValidatorNetwork
    fn on_view_change_message(&self, view_change: SignedViewChange) {
        debug!("[VIEW-CHANGE] Received view change from {}: {:#?}", self.peer.peer_address(), view_change.message);
        if !self.blockchain.is_in_current_epoch(view_change.message.block_number) {
            debug!("[VIEW-CHANGE] View change for old epoch: block_number={}", view_change.message.block_number);
        }
        else if let Some(validator_slots) = self.blockchain.get_current_validator_by_idx(view_change.signer_idx) {
            if view_change.verify(&validator_slots.public_key.uncompress_unchecked()) {
                self.notifier.read().notify(ValidatorAgentEvent::ViewChange {
                    view_change,
                    public_key: validator_slots.public_key.uncompress_unchecked().clone(),
                    slots: validator_slots.num_slots
                });
            }
            else {
                debug!("[VIEW-CHANGE] Invalid signature");
            }
        }
        else {
            debug!("[VIEW-CHANGE] Invalid validator index: {}", view_change.signer_idx);
        }
    }

    /// When a pbft block proposal is received
    fn on_pbft_proposal_message(&self, proposal: SignedPbftProposal) {
        let block_number = proposal.message.header.block_number;
        let view_number = proposal.message.header.view_number;

        debug!("[PBFT-PROPOSAL] Macro block proposal: block_number={}, view_number={}: {}", block_number, view_number, proposal.message.header.hash::<Blake2bHash>());

        if let Some((_, slot)) = self.blockchain.get_block_producer_at(block_number, view_number) {
            let public_key = &slot.public_key.uncompress_unchecked();

            // check the validity of the block
            let block = Block::Macro(MacroBlock {
                header: proposal.message.header.clone(),
                justification: None,
                extrinsics: None
            });
            if let Err(e) = self.blockchain.verify_macro_block_header(&proposal.message.header, proposal.message.view_change.as_ref()) {
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
                if view_change_proof.verify(&view_change, &self.blockchain.current_validators(), TWO_THIRD_VALIDATORS).is_err() {
                    debug!("[PBFT-PROPOSAL] Invalid view change proof: {:?}", view_change_proof);
                    return;
                }
            }


            self.notifier.read().notify(ValidatorAgentEvent::PbftProposal(proposal));
        }
        else {
            debug!("[PBFT-PROPOSAL] No pBFT leader");
        }
    }

    /// When a pbft prepare message is received, verify the signature and pass it to ValidatorNetwork
    fn on_pbft_prepare_message(&self, prepare: SignedPbftPrepareMessage) {
        debug!("[PBFT-PREPARE] Received prepare from {}: {}", self.peer.peer_address(), prepare.message.block_hash);
        if let Some(validator_slots) = self.blockchain.get_current_validator_by_idx(prepare.signer_idx) {
            if prepare.verify(&validator_slots.public_key.uncompress_unchecked()) {
                self.notifier.read().notify(ValidatorAgentEvent::PbftPrepare {
                    prepare,
                    public_key: validator_slots.public_key.uncompress_unchecked().clone(),
                    slots: validator_slots.num_slots
                });
            }
            else {
                warn!("[PBFT-PREPARE] Invalid signature: {:#?}", prepare);
                trace!("Public Key: {:?}", validator_slots.public_key);
            }
        }
        else {
            debug!("[PBFT-PREPARE] Invalid validator index: {}", prepare.signer_idx);
        }
    }

    /// When a pbft commit message is received, verify the signature and pass it to ValidatorNetwork
    fn on_pbft_commit_message(&self, commit: SignedPbftCommitMessage) {
        debug!("[PBFT-COMMIT] Received commit from {}: {:#?}", self.peer.peer_address(), commit.message);
        if let Some(validator_slots) = self.blockchain.get_current_validator_by_idx(commit.signer_idx) {
            if commit.verify(&validator_slots.public_key.uncompress_unchecked()) {
                self.notifier.read().notify(ValidatorAgentEvent::PbftCommit {
                    commit,
                    public_key: validator_slots.public_key.uncompress_unchecked().clone(),
                    slots: validator_slots.num_slots
                });
            }
            else {
                debug!("[PBFT-COMMIT] Invalid signature");
            }
        }
        else {
            warn!("[PBFT-COMMIT] Invalid validator index: {}", commit.signer_idx);
        }
    }
}
