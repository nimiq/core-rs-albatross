use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use bytes::Bytes;
use futures::{
    channel::mpsc,
    task::{noop_waker_ref, Context, Poll, Waker},
};
use libp2p::core::connection::ConnectionId;
use libp2p::core::Multiaddr;
use libp2p::swarm::{
    IntoProtocolsHandler, NetworkBehaviour, NetworkBehaviourAction, NotifyHandler, PollParameters,
};
use libp2p::{core::ConnectedPoint, PeerId};

use nimiq_network_interface::{
    message::MessageType, network::NetworkEvent, peer_map::ObservablePeerMap,
};

use crate::peer::Peer;

use super::handler::{HandlerInEvent, HandlerOutEvent, MessageHandler};

type MessageNetworkBehaviourAction = NetworkBehaviourAction<NetworkEvent<Peer>, MessageHandler>;

#[derive(Default)]
pub struct MessageBehaviour {
    events: VecDeque<MessageNetworkBehaviourAction>,

    pub(crate) peers: ObservablePeerMap<Peer>,

    message_receivers: HashMap<MessageType, mpsc::Sender<(Bytes, Arc<Peer>)>>,

    waker: Option<Waker>,
}

impl MessageBehaviour {
    pub fn new() -> Self {
        Self {
            ..Default::default()
        }
    }

    /// Buffers a `NetworkBehaviourAction` that should be emitted by the `poll` method on the next invocation.
    fn push_event(&mut self, event: MessageNetworkBehaviourAction) {
        self.events.push_back(event);
        self.wake();
    }

    fn wake(&self) {
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }

    /// Registers a receiver to receive from all peers. This will also make sure that any newly connected peer already
    /// has a receiver (a.k.a. message handler) registered before any messages can be received.
    ///
    /// # Note
    ///
    /// When a peer connects, this will be registered in its `MessageDispatch`. Thus you must not register a separate
    /// receiver with the peer.
    ///
    /// # Arguments
    ///
    ///  - `type_id`: The message type (e.g. `MessageType::new(200)` for `RequestBlockHashes`)
    ///  - `tx`: The sender through which the data of the messages is sent to the handler.
    ///
    /// # Panics
    ///
    /// Panics if a receiver was already registered for this message type.
    ///
    pub fn receive_from_all(&mut self, type_id: MessageType, tx: mpsc::Sender<(Bytes, Arc<Peer>)>) {
        if let Some(sender) = self.message_receivers.get_mut(&type_id) {
            let mut cx = Context::from_waker(noop_waker_ref());
            if let Poll::Ready(Ok(_)) = sender.poll_ready(&mut cx) {
                panic!(
                    "A receiver for message type {} is already registered",
                    type_id
                );
            } else {
                log::debug!(
                    "Removing stale sender from global message_receivers: TYPE_ID: {}",
                    &type_id
                );
                self.message_receivers.remove(&type_id);
            }
        }

        // add the receiver to the pre existing peers
        for peer in self.peers.get_peers() {
            let mut dispatch = peer.dispatch.lock();

            dispatch.remove_receiver_raw(type_id);
            dispatch.receive_multiple_raw(vec![(type_id, tx.clone())]);
        }

        // add the receiver to the globally defined map
        self.message_receivers.insert(type_id, tx);
    }
}

impl NetworkBehaviour for MessageBehaviour {
    type ProtocolsHandler = MessageHandler;
    type OutEvent = NetworkEvent<Peer>;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        MessageHandler::new()
    }

    fn addresses_of_peer(&mut self, _peer_id: &PeerId) -> Vec<Multiaddr> {
        vec![]
    }

    fn inject_connected(&mut self, _peer_id: &PeerId) {}

    fn inject_disconnected(&mut self, _peer_id: &PeerId) {}

    fn inject_connection_established(
        &mut self,
        peer_id: &PeerId,
        connection_id: &ConnectionId,
        endpoint: &ConnectedPoint,
        _failed_addresses: Option<&Vec<Multiaddr>>,
    ) {
        tracing::debug!(
            "Connection established: peer_id={}, connection_id={:?}, endpoint={:?}",
            peer_id,
            connection_id,
            endpoint
        );

        // Send an event to the handler that tells it if this is an inbound or outbound connection, and the registered
        // messages handlers, that receive from all peers.
        self.events
            .push_back(NetworkBehaviourAction::NotifyHandler {
                peer_id: *peer_id,
                handler: NotifyHandler::One(*connection_id),
                event: HandlerInEvent::PeerConnected {
                    peer_id: *peer_id,
                    outbound: endpoint.is_dialer(),
                    receive_from_all: self.message_receivers.clone(),
                },
            });
    }

    fn inject_connection_closed(
        &mut self,
        peer_id: &PeerId,
        connection_id: &ConnectionId,
        endpoint: &ConnectedPoint,
        _handler: <Self::ProtocolsHandler as IntoProtocolsHandler>::Handler,
    ) {
        tracing::debug!(
            "Connection closed: peer_id={}, connection_id={:?}, endpoint={:?}",
            peer_id,
            connection_id,
            endpoint
        );

        // If we still know this peer, remove it and emit an `PeerLeft` event to the swarm.
        if let Some(peer) = self.peers.remove(peer_id) {
            tracing::debug!("Peer disconnected: {:?}", peer);
            self.push_event(NetworkBehaviourAction::GenerateEvent(
                NetworkEvent::PeerLeft(peer),
            ));
        }
    }

    fn inject_event(
        &mut self,
        _peer_id: PeerId,
        _connection: ConnectionId,
        event: HandlerOutEvent,
    ) {
        match event {
            HandlerOutEvent::PeerJoined { peer } => {
                self.peers.insert(Arc::clone(&peer));
                self.push_event(NetworkBehaviourAction::GenerateEvent(
                    NetworkEvent::PeerJoined(peer),
                ));
            }
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
        _params: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<Self::OutEvent, Self::ProtocolsHandler>> {
        // Emit custom events.
        if let Some(event) = self.events.pop_front() {
            // tracing::trace!("MessageBehaviour::poll: Emitting event: {:?}", event);
            return Poll::Ready(event);
        }

        // Remember the waker and then return Pending
        if self.waker.is_none() {
            self.waker = Some(cx.waker().clone());
        }

        Poll::Pending
    }
}
