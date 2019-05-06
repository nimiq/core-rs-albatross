use std::sync::Arc;
use network_primitives::validator_info::{SignedValidatorInfo, ValidatorInfo, ValidatorId};
use network::Peer;
use utils::observer::{PassThroughNotifier, weak_passthru_listener};
use parking_lot::RwLock;
use bls::bls12_381::PublicKey;
use messages::Message;
use block_albatross::SignedViewChange;


pub enum ValidatorAgentEvent {
    ValidatorInfo(ValidatorInfo)
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
        let this_weak = Arc::downgrade(this);

        this.read().peer.channel.msg_notifier.validator_info.write()
            .register(weak_passthru_listener(Arc::downgrade(this), |this, signed_infos: Vec<SignedValidatorInfo>| {
            this.read().on_validator_infos(signed_infos);
        }));
        this.read().peer.channel.msg_notifier.view_change.write()
            .register(weak_passthru_listener( Arc::downgrade(this), |this, signed_view_change| {
            this.read().on_view_change(signed_view_change);
        }));
        // TODO: Register other listeners (pBFT messages)
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
            /*if *self.peer.peer_address() == signed_info.message.peer_address {
                debug!("Got validator info for peer: {}", self.peer.peer_address());
                self.validator_info = Some(signed_info.message);
            }*/
        }
    }

    fn on_view_change(&self, signed_view_change: SignedViewChange) {
        let signature_okay = signed_view_change.verify(unimplemented!());
        debug!("[VIEW-CHANGE] Received view change: {:#?}, signature_okay={}", signed_view_change.message, signature_okay);
        unimplemented!();
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
}
