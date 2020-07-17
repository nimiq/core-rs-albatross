use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::{executor, future, StreamExt};
use parking_lot::RwLock;

use block_albatross::{
    ForkProof, MultiSignature, PbftCommitMessage, PbftPrepareMessage, SignedPbftProposal,
    ViewChange,
};
use blockchain_albatross::Blockchain;
use bls::CompressedPublicKey;
use handel::update::LevelUpdateMessage;
use hash::{Blake2bHash, Hash};
use messages::ViewChangeProofMessage;
use network::connection::close_type::CloseType;
use network::Peer;
use network_interface::peer::Peer as PeerInterface;
use peer_address::address::PeerId;
use primitives::policy;
use utils::observer::{weak_passthru_listener, PassThroughNotifier};
use utils::rate_limit::RateLimit;

use crate::pool::{PushResult, ValidatorPool};
use crate::validator_info::{SignedValidatorInfo, SignedValidatorInfos, ValidatorInfo};

pub enum ValidatorAgentEvent {
    ValidatorInfos(Vec<SignedValidatorInfo>),
    ForkProof(Box<ForkProof>),
    ViewChange(Box<LevelUpdateMessage<MultiSignature, ViewChange>>),
    ViewChangeProof(Box<ViewChangeProofMessage>),
    PbftProposal(Box<SignedPbftProposal>),
    PbftPrepare(Box<LevelUpdateMessage<MultiSignature, PbftPrepareMessage>>),
    PbftCommit(Box<LevelUpdateMessage<MultiSignature, PbftCommitMessage>>),
}

pub struct ValidatorAgentState {
    pub(crate) validator_info: Option<SignedValidatorInfo>,
    pbft_proposal_limit: RateLimit,
    known_validators: BTreeSet<CompressedPublicKey>,
}

pub struct ValidatorAgent {
    pub(crate) peer: Arc<Peer>,
    pub(crate) blockchain: Arc<Blockchain>,
    pub(crate) state: RwLock<ValidatorAgentState>,
    validators: Weak<RwLock<ValidatorPool>>,
    pub notifier: RwLock<PassThroughNotifier<'static, ValidatorAgentEvent>>,
}

impl ValidatorAgent {
    pub fn new(
        peer: Arc<Peer>,
        blockchain: Arc<Blockchain>,
        validators: Weak<RwLock<ValidatorPool>>,
    ) -> Arc<Self> {
        let agent = Arc::new(Self {
            peer,
            blockchain,
            state: RwLock::new(ValidatorAgentState {
                validator_info: None,
                pbft_proposal_limit: RateLimit::new(5, Duration::from_secs(10)),
                known_validators: BTreeSet::new(),
            }),
            validators,
            notifier: RwLock::new(PassThroughNotifier::new()),
        });

        Self::init_listeners(&agent);

        agent
    }

    fn init_listeners(this: &Arc<Self>) {
        let weak = Arc::downgrade(this);
        let weak1 = Arc::downgrade(this);
        tokio::spawn(
            this.peer
                .receive::<SignedValidatorInfos>()
                .take_while(move |_| future::ready(weak.strong_count() > 0))
                .for_each(move |infos| {
                    if let Some(this) = weak1.upgrade() {
                        this.on_validator_infos(infos.into());
                    }
                    future::ready(())
                }),
        );

        // this.peer
        //     .channel
        //     .msg_notifier
        //     .validator_info
        //     .write()
        //     .register(weak_passthru_listener(
        //         Arc::downgrade(this),
        //         |this, signed_infos: Vec<SignedValidatorInfo>| {
        //             this.on_validator_infos(signed_infos);
        //         },
        //     ));
        this.peer
            .channel
            .msg_notifier
            .fork_proof
            .write()
            .register(weak_passthru_listener(
                Arc::downgrade(this),
                |this, fork_proof| {
                    this.on_fork_proof_message(fork_proof);
                },
            ));
        this.peer
            .channel
            .msg_notifier
            .pbft_proposal
            .write()
            .register(weak_passthru_listener(
                Arc::downgrade(this),
                |this, proposal| this.on_pbft_proposal_message(proposal),
            ));

        this.peer
            .channel
            .msg_notifier
            .pbft_prepare
            .write()
            .register(weak_passthru_listener(
                Arc::downgrade(this),
                |this, prepare| this.on_pbft_prepare_message(prepare),
            ));
        this.peer
            .channel
            .msg_notifier
            .pbft_commit
            .write()
            .register(weak_passthru_listener(
                Arc::downgrade(this),
                |this, commit| this.on_pbft_commit_message(commit),
            ));
        this.peer
            .channel
            .msg_notifier
            .view_change
            .write()
            .register(weak_passthru_listener(
                Arc::downgrade(this),
                |this, view_change| {
                    this.on_view_change_message(view_change);
                },
            ));
        this.peer
            .channel
            .msg_notifier
            .view_change_proof
            .write()
            .register(weak_passthru_listener(
                Arc::downgrade(this),
                |this, view_change_proof| {
                    this.on_view_change_proof(view_change_proof);
                },
            ));
    }

    /// When a list of validator infos is received, verify the signatures and notify
    fn on_validator_infos(&self, infos: Vec<SignedValidatorInfo>) {
        if infos.is_empty() {
            // peer send empty validator info set
            warn!(
                "Received empty validator info message from {}",
                self.peer.peer_address()
            );
            self.peer.channel.close(CloseType::EmptyValidatorInfo);
            return;
        }

        let num_infos = infos.len();
        let mut close_type: Option<CloseType> = None;
        let mut checked_infos = Vec::with_capacity(infos.len());

        let validators = match Weak::upgrade(&self.validators) {
            Some(validators) => validators,
            None => return,
        };
        let mut validators = validators.write();

        // TODO: Verify in parallel on a CpuPool
        for info in infos {
            trace!("Validator info: {:?}", info.message);

            match validators.push_validator_info(&info) {
                // Both will ban the peer. The peer should have checked that before relaying. We
                // ban because checking validator infos is expensive.
                // Should we abort here? The remaining validator infos could still be valid
                PushResult::InvalidPublicKey => {
                    warn!(
                        "Invalid public key in validator info from {}",
                        self.peer.peer_address()
                    );
                    close_type = Some(CloseType::InvalidPublicKeyInValidatorInfo)
                }
                PushResult::InvalidSignature => {
                    warn!(
                        "Invalid signature in validator info from {}",
                        self.peer.peer_address()
                    );
                    close_type = Some(CloseType::InvalidSignatureInValidatorInfo)
                }
                _ => checked_infos.push(info),
            }
        }

        drop(validators);

        if let Some(ban_type) = close_type {
            self.peer.channel.close(ban_type);
        }

        debug!(
            "Received {} unknown (total {}) validator infos",
            checked_infos.len(),
            num_infos
        );

        // Only notify validator, if there are any valid validator infos
        if !checked_infos.is_empty() {
            self.notifier
                .read()
                .notify(ValidatorAgentEvent::ValidatorInfos(checked_infos));
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

        let (slot, _) = self
            .blockchain
            .get_slot_owner_at(block_number, view_number, None);

        if let Err(e) = fork_proof.verify(&slot.public_key().uncompress_unchecked()) {
            debug!("[FORK-PROOF] Invalid signature in fork proof: {:?}", e);
            return;
        }

        self.notifier
            .read()
            .notify(ValidatorAgentEvent::ForkProof(Box::new(fork_proof)));
    }

    fn check_view_change_epoch(&self, view_change: &ViewChange) -> bool {
        let current_block_number = self.blockchain.block_number() + 1;
        let current_epoch = policy::epoch_at(current_block_number);

        let view_change_epoch = policy::epoch_at(view_change.block_number);

        if view_change_epoch == current_epoch {
            true
        } else {
            trace!("[VIEW-CHANGE] Ignoring view change message for a different epoch: current=#{}/{}, change_to=#{}/{}", current_block_number, current_epoch, view_change.block_number, view_change_epoch);
            false
        }
    }

    /// When a view change message is received, verify the signature and pass it to ValidatorNetwork
    fn on_view_change_message(
        &self,
        update_message: LevelUpdateMessage<MultiSignature, ViewChange>,
    ) {
        trace!(
            "[VIEW-CHANGE] Received: number={} update={:?} peer={}",
            update_message.tag,
            update_message.update,
            self.peer.peer_address()
        );

        if self.check_view_change_epoch(&update_message.tag) {
            self.notifier
                .read()
                .notify(ValidatorAgentEvent::ViewChange(Box::new(update_message)));
        }
    }

    fn on_view_change_proof(&self, proof: ViewChangeProofMessage) {
        trace!("[VIEW-CHANGE] Received proof: {:?}", proof);

        if self.check_view_change_epoch(&proof.view_change) {
            self.notifier
                .read()
                .notify(ValidatorAgentEvent::ViewChangeProof(Box::new(proof)));
        }
    }

    /// When a pbft block proposal is received
    fn on_pbft_proposal_message(&self, proposal: SignedPbftProposal) {
        if !self.state.write().pbft_proposal_limit.note_single() {
            warn!("Ignoring proposal - rate limit exceeded");
            return;
        }

        trace!("Received macro block proposal: {:?}", &proposal);

        let proposal_block = proposal.message.header.block_number;
        let current_block = self.blockchain.head().block_number();

        // Reject proposal if the blockchain is already longer
        if proposal_block <= current_block {
            trace!(
                "[PBFT-PROPOSAL] Ignoring old proposal: {:?}",
                proposal.message.header
            );
            return;
        }

        // Reject proposal if it lies in another epoch
        if policy::epoch_at(proposal_block) != policy::epoch_at(current_block) {
            warn!(
                "[PBFT-PROPOSAL] Ignoring proposal in another epoch: {}",
                proposal.message.header.hash::<Blake2bHash>()
            );
            return;
        }

        self.notifier
            .read()
            .notify(ValidatorAgentEvent::PbftProposal(Box::new(proposal)));
    }

    /// When a pbft prepare message is received, verify the signature and pass it to ValidatorNetwork
    /// TODO: The validator network could just register this it-self
    fn on_pbft_prepare_message(
        &self,
        level_update: LevelUpdateMessage<MultiSignature, PbftPrepareMessage>,
    ) {
        trace!(
            "[PBFT-PREPARE] Received: block_hash={} update={:?} peer={}",
            level_update.tag.block_hash,
            level_update.update,
            self.peer.peer_address()
        );

        self.notifier
            .read()
            .notify(ValidatorAgentEvent::PbftPrepare(Box::new(level_update)));
    }

    /// When a pbft commit message is received, verify the signature and pass it to ValidatorNetwork
    /// FIXME This will verify a commit message with the current validator set, not with the one for
    /// which this commit is for.
    fn on_pbft_commit_message(
        &self,
        level_update: LevelUpdateMessage<MultiSignature, PbftCommitMessage>,
    ) {
        trace!(
            "[PBFT-COMMIT] Received: block_hash={} update={:?} peer={}",
            level_update.tag.block_hash,
            level_update.update,
            self.peer.peer_address()
        );

        self.notifier
            .read()
            .notify(ValidatorAgentEvent::PbftCommit(Box::new(level_update)));
    }

    pub fn validator_info(&self) -> Option<ValidatorInfo> {
        self.state
            .read()
            .validator_info
            .as_ref()
            .map(|signed| signed.message.clone())
    }

    pub fn public_key(&self) -> Option<CompressedPublicKey> {
        self.state
            .read()
            .validator_info
            .as_ref()
            .map(|info| info.message.public_key.clone())
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer.peer_address().peer_id.clone()
    }

    pub fn send_validator_infos(&self, infos: &Vec<SignedValidatorInfo>) {
        let mut state = self.state.write();

        let num_infos = infos.len(); // DEBUGGING

        let unknown_infos = infos
            .iter()
            .filter(|info| !state.known_validators.contains(&info.message.public_key))
            .cloned()
            .collect::<Vec<SignedValidatorInfo>>();

        // early return, if there are no unknown
        if unknown_infos.is_empty() {
            return;
        }

        // add unknown validators to known set
        for info in &unknown_infos {
            state
                .known_validators
                .insert(info.message.public_key.clone());
        }

        // send unknown infos
        debug!(
            "Sending {} unknown validator infos (out of {}) to {}",
            unknown_infos.len(),
            num_infos,
            self.peer.peer_address()
        );

        executor::block_on(self.peer.send(&SignedValidatorInfos(unknown_infos))).ok();
    }
}

impl fmt::Debug for ValidatorAgent {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(
            f,
            "ValidatorAgent {{ peer_id: {}, public_key: {:?} }}",
            self.peer_id(),
            self.public_key()
        )
    }
}
