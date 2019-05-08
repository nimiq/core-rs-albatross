use std::sync::Arc;
use network_primitives::validator_info::{SignedValidatorInfo, ValidatorInfo, ValidatorId};
use network::Peer;
use utils::observer::{PassThroughNotifier, weak_passthru_listener};
use parking_lot::RwLock;
use bls::bls12_381::PublicKey;
use block_albatross::{SignedViewChange, SignedPbftPrepareMessage, SignedPbftCommitMessage,
                      MacroHeader, SignedPbftProposal, ViewChange};
use hash::{Hash, Blake2bHash};
use primitives::policy::TWO_THIRD_VALIDATORS;


pub enum ValidatorAgentEvent {
    ValidatorInfo(ValidatorInfo),
    ViewChange { view_change: SignedViewChange, public_key: PublicKey, slots: u16 },
    PbftProposal(SignedPbftProposal),
    PbftPrepare { prepare: SignedPbftPrepareMessage, public_key: PublicKey, slots: u16 },
    PbftCommit { commit: SignedPbftCommitMessage, public_key: PublicKey, slots: u16 }
}

pub struct ValidatorAgent {
    peer: Arc<Peer>,
    validator_info: Option<ValidatorInfo>,
    pub notifier: RwLock<PassThroughNotifier<'static, ValidatorAgentEvent>>,
}

impl ValidatorAgent {
    pub fn new(peer: Arc<Peer>) -> Arc<RwLock<Self>> {
        let agent = Arc::new(RwLock::new(Self {
            peer: Arc::clone(&peer),
            validator_info: None,
            notifier: RwLock::new(PassThroughNotifier::new()),
        }));

        Self::init_listeners(&agent);

        agent
    }

    fn init_listeners(this: &Arc<RwLock<Self>>) {
        this.read().peer.channel.msg_notifier.validator_info.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, signed_infos: Vec<SignedValidatorInfo>| {
                this.read().on_validator_infos(signed_infos);
            }));
        this.read().peer.channel.msg_notifier.view_change.write()
            .register(weak_passthru_listener( Arc::downgrade(this), |this, signed_view_change| {
                this.read().on_view_change_message(signed_view_change);
            }));
        this.read().peer.channel.msg_notifier.pbft_proposal.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, block| {
                this.read().on_pbft_proposal_message(block);
            }));
        this.read().peer.channel.msg_notifier.pbft_prepare.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, prepare| {
                this.read().on_pbft_prepare_message(prepare);
            }));
        this.read().peer.channel.msg_notifier.pbft_commit.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, commit| {
                this.read().on_pbft_commit_message(commit);
            }));
    }

    fn on_validator_infos(&self, signed_infos: Vec<SignedValidatorInfo>) {
        debug!("[VALIDATOR-INFO] contains {} validator infos", signed_infos.len());
        for signed_info in signed_infos {
            let signature_okay = signed_info.verify(&signed_info.message.public_key);
            debug!("[VALIDATOR-INFO] {:#?}, signature_okay={}", signed_info.message, signature_okay);
            if !signature_okay {
                continue;
            }
            self.notifier.read().notify(ValidatorAgentEvent::ValidatorInfo(signed_info.message));
        }
    }

    /// When a view change message is received, verify the signature and pass it to ValidatorNetwork
    fn on_view_change_message(&self, view_change: SignedViewChange) {
        debug!("[VIEW-CHANGE] Received view change from {}: {:#?}", self.peer.peer_address(), view_change.message);
        if !self.in_current_epoch(view_change.message.block_number) {
            debug!("[VIEW-CHANGE] View change for old epoch: block_number={}", view_change.message.block_number);
        }
        else if let Some((public_key, slots)) = self.get_validator_slots(view_change.pk_idx) {
            if view_change.verify(&public_key) {
                self.notifier.read().notify(ValidatorAgentEvent::ViewChange {
                    view_change,
                    public_key,
                    slots
                });
            }
            else {
                debug!("[VIEW-CHANGE] Invalid signature");
            }
        }
        else {
            debug!("[VIEW-CHANGE] Invalid validator index: {}", view_change.pk_idx);
        }
    }

    /// When a pbft block proposal is received
    /// TODO: check correctness of proposed block (i.e. parent hashes, timestamp, seed)
    fn on_pbft_proposal_message(&self, proposal: SignedPbftProposal) {
        debug!("[PBFT-PROPOSAL] Macro block proposal: {:#?}", proposal.message);
        if let Some(public_key) = self.get_pbft_leader() {
            // check the signature of the proposal
            // XXX We ignore the `pk_idx` field in the `SignedMessage`
            if !proposal.verify(&public_key) {
                debug!("[PBFT-PROPOSAL] Invalid signature");
                return
            }

            // check the view change proof
            if let Some(view_change_proof) = &proposal.message.view_change {
                let view_change = ViewChange {
                    block_number: self.get_block_number(),
                    new_view_number: proposal.message.view_number,
                };
                if !view_change_proof.verify(&view_change, TWO_THIRD_VALIDATORS) {
                    debug!("[PBFT-PROPOSAL] Invalid view change proof: {:?}", view_change_proof);
                    return
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
        debug!("[PBFT-PREPARE] Received prepare from {}: {:#?}", self.peer.peer_address(), prepare.message);
        // TODO: check that the block_hash is the hash of the proposed block
        if let Some((public_key, slots)) = self.get_validator_slots(prepare.pk_idx) {
            if prepare.verify(&public_key) {
                self.notifier.read().notify(ValidatorAgentEvent::PbftPrepare {
                    prepare,
                    public_key,
                    slots
                });
            }
            else {
                debug!("[PBFT-PREPARE] Invalid signature");
            }
        }
        else {
            debug!("[PBFT-PREPARE] Invalid validator index: {}", prepare.pk_idx);
        }
    }

    /// When a pbft commit message is received, verify the signature and pass it to ValidatorNetwork
    fn on_pbft_commit_message(&self, commit: SignedPbftCommitMessage) {
        debug!("[PBFT-COMMIT] Received commit from {}: {:#?}", self.peer.peer_address(), commit.message);
        // TODO: check that the block_hash is the hash of the proposed block
        if let Some((public_key, slots)) = self.get_validator_slots(commit.pk_idx) {
            if commit.verify(&public_key) {
                self.notifier.read().notify(ValidatorAgentEvent::PbftCommit {
                    commit,
                    public_key,
                    slots
                });
            }
            else {
                debug!("[PBFT-COMMIT] Invalid signature");
            }
        }
        else {
            warn!("[PBFT-COMMIT] Invalid validator index: {}", commit.pk_idx);
        }
    }

    pub fn set_info(&mut self, validator_info: ValidatorInfo) {
        if *self.peer.peer_address() == validator_info.peer_address {
            self.validator_info = Some(validator_info);
        }
        else {
            warn!("Tried to set ValidatorInfo in ValidatorAgent with different peer addresses.");
        }
    }

    pub fn get_public_key(&self) -> Option<&PublicKey> {
        self.validator_info.as_ref().map(|info| &info.public_key)
    }

    pub fn get_validator_id(&self) -> Option<&ValidatorId> {
        self.validator_info.as_ref().map(|info| &info.validator_id)
    }

    pub fn get_pk_idx(&self) -> Option<u16> {
        self.validator_info.as_ref().and_then(|info| info.pk_idx)
    }

    pub fn is_active(&self) -> bool {
        self.validator_info.as_ref()
            .map(|info| info.pk_idx.is_some())
            .unwrap_or(false)
    }


    /// TODO: Those methods should be implemented somewhere else

    /// get a validator's public key and number of slots
    fn get_validator_slots(&self, pk_idx: u16) -> Option<(PublicKey, u16)> {
        unimplemented!("get public key, slots for pk_idx");
    }

    /// check if a block number is in the current epoch
    fn in_current_epoch(&self, block_number: u32) -> bool {
        unimplemented!("check if block number is in current epoch");
    }

    fn get_pbft_leader(&self) -> Option<&PublicKey> {
        unimplemented!()
    }

    fn get_block_number(&self) -> u32 {
        unimplemented!()
    }
}
