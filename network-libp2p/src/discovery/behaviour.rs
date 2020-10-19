use libp2p::{
    swarm::{NetworkBehaviour, PollParameters, NetworkBehaviourAction},
    core::connection::ConnectionId,
    PeerId, Multiaddr,
};
use futures::task::{Context, Poll};

use super::{
    event::DiscoveryEvent,
    handler::DiscoveryHandler,
    protocol::DiscoveryMessage,

};


pub struct Discovery {
}


impl NetworkBehaviour for Discovery {
    type ProtocolsHandler = DiscoveryHandler;
    type OutEvent = DiscoveryEvent;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        unimplemented!()
    }

    fn addresses_of_peer(&mut self, peer_id: &PeerId) -> Vec<Multiaddr> {
        unimplemented!()
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        unimplemented!()
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        unimplemented!()
    }

    fn inject_event(&mut self, peer_id: PeerId, connection: ConnectionId, event: DiscoveryMessage) {
        unimplemented!()
    }

    fn poll(&mut self, cx: &mut Context, params: &mut impl PollParameters) -> Poll<NetworkBehaviourAction<(), DiscoveryEvent>> {
        unimplemented!()
    }
}