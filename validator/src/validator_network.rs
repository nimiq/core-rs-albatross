use std::sync::{Arc, Weak};

use network::{Network, NetworkConfig, NetworkEvent, Peer};

pub struct ValidatorNetwork {
    pub network: Arc<Network>,
}

impl ValidatorNetwork {
    pub fn new(network: Arc<Network>) -> Arc<Self> {
        let this = Arc::new(ValidatorNetwork {
            network
        });

        ValidatorNetwork::init_listeners(&this);
        this
    }

    fn init_listeners(this: &Arc<ValidatorNetwork>) {
        let weak = Arc::downgrade(this);
        this.network.notifier.write().register(move |e: NetworkEvent| {
            let this = upgrade_weak!(weak);
            match e {
                NetworkEvent::PeerJoined(peer) => this.on_peer_joined(peer),
                NetworkEvent::PeerLeft(peer) => this.on_peer_left(peer),
                _ => {}
            }
        });
    }

    pub fn on_peer_joined(&self, peer: Peer) {
        unimplemented!();
    }

    pub fn on_peer_left(&self, peer: Peer) {
        unimplemented!();
    }
}
