use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use futures::channel::mpsc;
use futures::task::{Context, Poll};
use futures::{ready, StreamExt};
use libp2p::core::connection::ConnectionId;
use libp2p::core::Multiaddr;
use libp2p::swarm::{
    NetworkBehaviour, NetworkBehaviourAction, NotifyHandler, PollParameters, ProtocolsHandler,
};
use libp2p::PeerId;

use network_interface::network::NetworkEvent;

use crate::handler::{NimiqHandler, NimiqHandlerAction};
use crate::peer::{Peer, PeerAction};

pub struct NimiqBehaviour {
    peers: HashMap<PeerId, Arc<Peer>>,
    events: VecDeque<NetworkEvent<Peer>>,
    peer_tx: mpsc::Sender<PeerAction>,
    peer_rx: mpsc::Receiver<PeerAction>,
}

impl NimiqBehaviour {
    pub fn new() -> Self {
        let (peer_tx, peer_rx) = mpsc::channel(4096);
        Self {
            peers: HashMap::new(),
            events: VecDeque::new(),
            peer_tx,
            peer_rx,
        }
    }
}

impl NetworkBehaviour for NimiqBehaviour {
    type ProtocolsHandler = NimiqHandler;
    type OutEvent = NetworkEvent<Peer>;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        NimiqHandler::new()
    }

    fn addresses_of_peer(&mut self, _peer_id: &PeerId) -> Vec<Multiaddr> {
        Vec::new()
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        println!("Connected to {}", peer_id);

        let peer = Arc::new(Peer::new(peer_id.clone(), self.peer_tx.clone()));
        self.peers
            .insert(peer_id.clone(), Arc::clone(&peer))
            .map(|p| panic!("Duplicate peer {}", p.id));

        self.events.push_back(NetworkEvent::PeerJoined(peer));
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        println!("Disconnected from {}", peer_id);

        let peer = self
            .peers
            .remove(peer_id)
            .expect("Unknown peer disconnected");

        self.events.push_back(NetworkEvent::PeerLeft(peer));
    }

    fn inject_event(
        &mut self,
        peer_id: PeerId,
        connection: ConnectionId,
        msg: <Self::ProtocolsHandler as ProtocolsHandler>::OutEvent,
    ) {
        println!(
            "Message with len {} bytes received from peerId {} on connId {:?}",
            msg.len(),
            peer_id,
            connection,
        );

        let peer = self
            .peers
            .get(&peer_id)
            .expect("Message received from unknown peer");
        peer.dispatch_inbound_msg(msg);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<NimiqHandlerAction, NetworkEvent<Peer>>> {
        // Emit custom events.
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event));
        }

        // Drive inbound message dispatch futures to completion.
        // if let Some(dispatch) = self.dispatching.front_mut() {
        //     if let Poll::Ready(_) = dispatch.poll_unpin(cx) {
        //         self.dispatching.pop_front();
        //     }
        // }

        // Notify handlers for outbound messages.
        match ready!(self.peer_rx.poll_next_unpin(cx)) {
            Some(PeerAction::Message(peer_id, msg)) => {
                println!("Received message action for {}", peer_id);
                Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                    peer_id,
                    handler: NotifyHandler::Any,
                    event: NimiqHandlerAction::Message(msg),
                })
            }
            Some(PeerAction::Close(peer_id)) => {
                println!("Received close action for {}", peer_id);
                Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                    peer_id,
                    handler: NotifyHandler::All,
                    event: NimiqHandlerAction::Close,
                })
            }
            None => Poll::Pending,
        }
    }
}
