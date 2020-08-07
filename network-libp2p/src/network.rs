use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use futures::channel::mpsc;
use futures::task::{Context, Poll};
use futures::{executor, future, ready, Future, SinkExt, StreamExt};
use libp2p::core::Multiaddr;
use libp2p::swarm::SwarmBuilder;
use libp2p::{PeerId, Swarm};
use parking_lot::{Mutex, RwLock};
use tokio::sync::broadcast;

use network_interface::network::{Network as NetworkInterface, NetworkEvent};

use crate::behaviour::NimiqBehaviour;
use crate::peer::Peer;

#[derive(Debug)]
enum SwarmAction {
    Dial(PeerId),
    DialAddr(Multiaddr),
}

struct SwarmTask {
    swarm: Swarm<NimiqBehaviour>,
    event_tx: broadcast::Sender<NetworkEvent<Peer>>,
    action_rx: mpsc::Receiver<SwarmAction>,
}

impl SwarmTask {
    fn new(
        swarm: Swarm<NimiqBehaviour>,
        event_tx: broadcast::Sender<NetworkEvent<Peer>>,
        action_rx: mpsc::Receiver<SwarmAction>,
    ) -> Self {
        Self {
            swarm,
            event_tx,
            action_rx,
        }
    }
}

impl SwarmTask {
    fn perform_action(&mut self, action: SwarmAction) {
        println!("Performing swarm action: {:?}", action);
        match action {
            SwarmAction::Dial(peer_id) => Swarm::dial(&mut self.swarm, &peer_id)
                .map_err(|err| println!("Failed to dial peer {}: {:?}", peer_id, err)),
            SwarmAction::DialAddr(addr) => Swarm::dial_addr(&mut self.swarm, addr)
                .map_err(|err| println!("Failed to dial addr: {:?}", err)),
        }
        // Error handling?
        .unwrap_or(())
    }
}

impl Future for SwarmTask {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        println!(
            "SwarmTask.poll(): receiver_count={}",
            self.event_tx.receiver_count()
        );

        // The network instance that spawn this task is subscribed to the events channel.
        // If the receiver count drops to zero, the network has gone away and we stop this task.
        if self.event_tx.receiver_count() < 1 {
            return Poll::Ready(());
        }

        // Execute pending swarm actions.
        while let Poll::Ready(action) = self.action_rx.poll_next_unpin(cx) {
            match action {
                Some(action) => self.perform_action(action),
                None => return Poll::Ready(()), // Network is gone, terminate.
            }
        }

        // Poll the swarm.
        match ready!(self.swarm.poll_next_unpin(cx)) {
            Some(event) => {
                // Dispatch swarm event on network event broadcast channel.
                if self.event_tx.send(event).is_ok() {
                    // Keep the task alive.
                    Poll::Pending
                } else {
                    // Event dispatch can still fail if the network was dropped after the check above.
                    Poll::Ready(())
                }
            }
            None => {
                // Swarm has terminated.
                info!("Swarm terminated");
                Poll::Ready(())
            }
        }
    }
}

pub struct Network {
    peers: Arc<RwLock<HashMap<PeerId, Arc<Peer>>>>,
    local_peer_id: PeerId,
    event_tx: broadcast::Sender<NetworkEvent<Peer>>,
    action_tx: Mutex<mpsc::Sender<SwarmAction>>,
}

impl Network {
    // TODO add proper config
    pub fn new(listen_addr: Multiaddr) -> Self {
        let (event_tx, events_rx) = broadcast::channel::<NetworkEvent<Peer>>(64);
        let (action_tx, action_rx) = mpsc::channel(16);

        let peers = Arc::new(RwLock::new(HashMap::new()));
        tokio::spawn(Self::new_network_task(events_rx, &peers));

        let swarm = Self::new_swarm(listen_addr);
        let local_peer_id = Swarm::local_peer_id(&swarm).clone();
        tokio::spawn(SwarmTask::new(swarm, event_tx.clone(), action_rx));

        Self {
            peers,
            local_peer_id,
            event_tx,
            action_tx: Mutex::new(action_tx),
        }
    }

    fn new_swarm(listen_addr: Multiaddr) -> Swarm<NimiqBehaviour> {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        let local_peer_id = PeerId::from(keypair.public());
        let transport = libp2p::build_tcp_ws_secio_mplex_yamux(keypair).unwrap();
        let behaviour = NimiqBehaviour::new();

        // TODO add proper config
        let mut swarm = SwarmBuilder::new(transport, behaviour, local_peer_id)
            .incoming_connection_limit(5)
            .outgoing_connection_limit(2)
            .peer_connection_limit(1)
            .build();
        Swarm::listen_on(&mut swarm, listen_addr).expect("Failed to listen on provided address");
        swarm
    }

    fn new_network_task(
        event_rx: broadcast::Receiver<NetworkEvent<Peer>>,
        peers: &Arc<RwLock<HashMap<PeerId, Arc<Peer>>>>,
    ) -> impl Future<Output = ()> {
        let peers_weak1 = Arc::downgrade(peers);
        let peers_weak2 = Arc::downgrade(peers);
        event_rx
            .take_while(move |event| future::ready(event.is_ok() && peers_weak1.strong_count() > 0))
            .for_each(move |event| {
                // We check for event.is_ok() in take_while.
                let event = event.unwrap();
                if let Some(peers) = peers_weak2.upgrade() {
                    let mut peers = peers.write();
                    match event {
                        NetworkEvent::PeerJoined(peer) => peers
                            .insert(peer.id.clone(), peer)
                            .map_or((), |_| panic!("Duplicate peer")),
                        NetworkEvent::PeerLeft(peer) => peers
                            .remove(&peer.id)
                            .map(|_| ())
                            .expect("Unknown peer disconnected"),
                    }
                }
                future::ready(())
            })
    }

    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    pub fn dial(&self, peer_id: PeerId) {
        // TODO make async? error handling
        executor::block_on(self.action_tx.lock().send(SwarmAction::Dial(peer_id))).unwrap_or(())
    }

    pub fn dial_addr(&self, addr: Multiaddr) {
        // TODO make async? handling
        executor::block_on(self.action_tx.lock().send(SwarmAction::DialAddr(addr))).unwrap_or(())
    }
}

impl NetworkInterface for Network {
    type PeerType = Peer;

    fn get_peers(&self) -> Vec<Arc<Self::PeerType>> {
        self.peers.read().values().cloned().collect()
    }

    fn get_peer(&self, peer_id: &PeerId) -> Option<Arc<Self::PeerType>> {
        self.peers.read().get(peer_id).cloned()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<NetworkEvent<Self::PeerType>> {
        self.event_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures::task::{Context, Poll};
    use futures::{executor, future, FutureExt, StreamExt};
    use libp2p::core::Multiaddr;
    use libp2p::{PeerId, Swarm};
    use parking_lot::RwLock;

    use beserial::{Deserialize, Serialize};
    use network_interface::message::Message;
    use network_interface::network::{Network as NetworkInterface, NetworkEvent};
    use network_interface::peer::{CloseReason, Peer};

    use crate::network::Network;

    #[derive(Deserialize, Serialize)]
    struct TestMessage {
        id: u32,
    }
    impl Message for TestMessage {
        const TYPE_ID: u64 = 42;
    }

    #[tokio::test]
    async fn test() {
        let addr1 = "/ip4/127.0.0.1/tcp/10001".parse::<Multiaddr>().unwrap();
        let addr2 = "/ip4/127.0.0.1/tcp/10002".parse::<Multiaddr>().unwrap();

        let mut net1 = Network::new(addr1.clone());
        let mut net2 = Network::new(addr2.clone());

        // FIXME dial_addr only works from net2 to net1 but not the other way around... why?
        net2.dial_addr(addr1);

        let mut events1 = net1.subscribe_events();
        let mut events2 = net2.subscribe_events();

        future::join(events1.next(), events2.next()).await;

        assert_eq!(net1.get_peers().len(), 1);
        assert_eq!(net2.get_peers().len(), 1);

        let peer2 = net1.get_peer(net2.local_peer_id()).unwrap();
        let peer1 = net2.get_peer(net1.local_peer_id()).unwrap();

        let mut msg_stream = peer1.receive::<TestMessage>();
        peer2.send(&TestMessage { id: 4711 }).await.unwrap();

        let msg = msg_stream.next().await.unwrap();
        assert_eq!(msg.id, 4711);

        peer2.close(CloseReason::Other).await;

        future::join(events1.next(), events2.next()).await;

        assert_eq!(net1.get_peers().len(), 0);
        assert_eq!(net2.get_peers().len(), 0);
    }
}
