use std::sync::{Arc, Weak};
use network::{Network, NetworkConfig, NetworkEvent, Peer};
use std::collections::HashMap;
use parking_lot::RwLock;
use crate::validator_agent::{ValidatorAgent, ValidatorAgentEvent};
use network_primitives::validator_info::ValidatorInfo;
use utils::observer::{PassThroughNotifier, weak_passthru_listener};
use messages::Message;
use std::collections::btree_map::BTreeMap;
use block_albatross::{
    ViewChange, SignedViewChange, ViewChangeProof,
    SignedPbftPrepareMessage, SignedPbftCommitMessage, PbftProof,
    signed,
};
use consensus::Consensus;
use bls::bls12_381::PublicKey;



pub struct ValidatorNetwork {
    network: Arc<Network>,
    validator_agents: HashMap<Arc<Peer>, Arc<RwLock<ValidatorAgent>>>,

    /// maps (view-change-number, block-number) to the proof that is being aggregated
    /// clear after macro block
    view_changes: BTreeMap<ViewChange, ViewChangeProof>,

    /// the current pbft proof (exists between proposal and macro block proofed)
    pbft_proof: PbftProof,

    /// The signature threshold for view-changes and pbft proofs
    threshold: u16,

    self_weak: Weak<RwLock<ValidatorNetwork>>,
    pub notifier: RwLock<PassThroughNotifier<'static, NetworkEvent>>,
}

impl ValidatorNetwork {
    pub fn new(network: Arc<Network>, consensus: Arc<Consensus>) -> Arc<RwLock<Self>> {
        let this = Arc::new(RwLock::new(ValidatorNetwork {
            network,
            validator_agents: HashMap::new(),
            view_changes: BTreeMap::new(),
            pbft_proof: PbftProof::new(),
            // XXX For view change: threshold = (2 * n + 3) / 3 - which equals ceil(2f + 1) where n = 3f + 1
            threshold: unimplemented!("Select number of slots * 2/3"),
            self_weak: Weak::new(),
            notifier: RwLock::new(PassThroughNotifier::new()),
        }));

        ValidatorNetwork::init_listeners(&this);
        this
    }

    fn init_listeners(this: &Arc<RwLock<ValidatorNetwork>>) {
        this.write().self_weak = Arc::downgrade(this);
        let weak = Arc::downgrade(this);

        this.read().network.notifier.write().register(move |e: &NetworkEvent| {
            let this = upgrade_weak!(weak);
            match e {
                NetworkEvent::PeerJoined(peer) => this.write().on_peer_joined(peer),
                NetworkEvent::PeerLeft(peer) => this.write().on_peer_left(peer),
                _ => {}
            }
        });
    }

    fn on_peer_joined(&mut self, peer: &Arc<Peer>) {
        if peer.peer_address().services.is_validator() {
            let agent = ValidatorAgent::new(Arc::clone(peer));
            self.validator_agents.insert(Arc::clone(peer), Arc::clone(&agent));

            let weak = Weak::clone(&self.self_weak);
            agent.read().notifier.write().register(weak_passthru_listener(Weak::clone(&self.self_weak), |this, event| {
                match event {
                    ValidatorAgentEvent::ValidatorInfo(info) => this.write().on_validator_info(info),
                }
            }));
        }
    }

    fn on_peer_left(&mut self, peer: &Arc<Peer>) {
        self.validator_agents.remove(peer);
    }

    fn on_validator_info(&mut self, info: ValidatorInfo) {
        unimplemented!()
    }

    /// When a view change was received by a ValidatorAgent
    fn on_view_change_message(&mut self, view_change: SignedViewChange) {
        if let Some((public_key, slots)) = self.get_validator_slots(view_change.pk_idx) {
            // TODO: I'd like to have the signature verification in the ValidatorAgent, but it
            //       currently doesn't have access to the validator list. This might change later.
            if view_change.verify(&public_key) {
                self.commit_view_change(view_change, &public_key, slots);
            }
            else {
                info!("Invalid view change message")
            }
        }
        else {
            warn!("Invalid validator index: {}", view_change.pk_idx);
        }
    }

    /// Commit a view change to the proofs being build and relay it if it's new
    pub fn commit_view_change(&mut self, view_change: SignedViewChange, public_key: &PublicKey, slots: u16) {
        // get the proof with the specific block number and view change number
        // if it doesn't exist, create a new one.
        let proof = self.view_changes.entry(view_change.message.clone())
            .or_insert_with(|| ViewChangeProof::new());

        // if we have enough signatures, notify listeners
        if proof.verify(&view_change.message, self.threshold) {
            unimplemented!("Notify that we got a valid view change proof")
        }

        // Aggregate signature - if it wasn't included yet, relay it
        if !proof.add_signature(&public_key, slots, &view_change) {
            // TODO: don't clone, broadcast after verify
            self.broadcast_active(Message::ViewChange(Box::new(view_change.clone())));
        }
    }

    fn on_pbft_proposal_message(&mut self, block: ()) {
        unimplemented!("Notify that we have a view change proof")
    }

    /// When a pBFT prepare was received by a ValidatorAgent
    fn on_pbft_prepare_message(&mut self, prepare: SignedPbftPrepareMessage) {
        if let Some((public_key, slots)) = self.get_validator_slots(prepare.pk_idx) {
            if prepare.verify(&public_key) {
                self.commit_pbft_prepare(prepare, &public_key, slots);
            }
        }
        else {
            warn!("Invalid validator index: {}", prepare.pk_idx);
        }
    }

    /// Commit a pBFT prepare
    pub fn commit_pbft_prepare(&mut self, prepare: SignedPbftPrepareMessage, public_key: &PublicKey, slots: u16) {
        // aggregate prepare signature - if new, relay
        if !self.pbft_proof.add_prepare_signature(&public_key, slots, &prepare) {
            self.broadcast_active(Message::PbftPrepare(Box::new(prepare.clone())));
        }

        // notify if we reach threshold on prepare to begin commit
        if self.pbft_proof.prepare.verify(&prepare.message, self.threshold) {
            unimplemented!("Notify that we have a pBFT prepare proof")
        }

        // NOTE: It might happen that we receive the prepare message after the commit, due to
        //       gossiping. So we have to verify here too.
        if self.pbft_proof.verify(prepare.message.block_hash, self.threshold) {
            unimplemented!("Notify that we have a pBFT proof")
        }
    }

    /// When a pBFT commit was received by a ValidatorAgent
    fn on_pbft_commit_message(&mut self, commit: SignedPbftCommitMessage) {
        if let Some((public_key, slots)) = self.get_validator_slots(commit.pk_idx) {
            if commit.verify(&public_key) {
                self.commit_pbft_commit(commit, &public_key, slots);
            }
        }
        else {
            warn!("Invalid validator index: {}", commit.pk_idx);
        }
    }

    /// Commit a pBFT commit
    pub fn commit_pbft_commit(&mut self, commit: SignedPbftCommitMessage, public_key: &PublicKey, slots: u16) {
        // aggregate commit signature - if new, relay
        if !self.pbft_proof.add_commit_signature(&public_key, slots, &commit) {
            self.broadcast_active(Message::PbftCommit(Box::new(commit.clone())))
        }

        if self.pbft_proof.verify(commit.message.block_hash, self.threshold) {
            unimplemented!("Notify that we have a pBFT proof")
        }
    }

    // TODO: register in consensus for macro blocks
    pub fn on_finality(&mut self) {
        debug!("Clearing view change and pBFT proof");
        self.view_changes.clear();
        self.pbft_proof.clear();
    }

    fn broadcast_active(&self, msg: Message) {
        for (peer, agent) in self.validator_agents.iter() {
            // TODO: in order to do this without cloning, we need to pass references all the way
            // to the websocket. We can easily just serialize a reference.
            peer.channel.send_or_close(msg.clone());
        }
    }

    fn get_validator_slots(&self, pk_idx: u16) -> Option<(PublicKey, u16)> {
        unimplemented!("get public key, slots for pk_idx");
    }
}
