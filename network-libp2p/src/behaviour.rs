use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use libp2p::NetworkBehaviour;
use libp2p::swarm::NetworkBehaviourEventProcess;

use network_interface::network::NetworkEvent;

use crate::limit::LimitBehaviour;
use crate::message::MessageBehaviour;
use crate::peer::Peer;

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "NetworkEvent<Peer>")]
pub struct NimiqBehaviour {
    pub message_behaviour: MessageBehaviour,
    pub limit_behaviour: LimitBehaviour,

    #[behaviour(ignore)]
    pub events: VecDeque<NetworkEvent<Peer>>,
}

impl NetworkBehaviourEventProcess<NetworkEvent<Peer>> for NimiqBehaviour {
    fn inject_event(&mut self, event: NetworkEvent<Peer>) {
        match event {
            NetworkEvent::PeerJoined(peer) => {
                self.limit_behaviour.peers
                    .insert(peer.id.clone(), Arc::clone(&peer))
                    .map(|p| panic!("Duplicate peer {}", p.id));
                self.events.push_back(NetworkEvent::PeerJoined(peer));
            },
            NetworkEvent::PeerLeft(peer) => {
                self.events.push_back(NetworkEvent::PeerLeft(peer));
            },
        }
    }
}
