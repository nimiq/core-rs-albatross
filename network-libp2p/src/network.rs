use std::collections::HashMap;
use std::sync::Arc;

use futures::task::{Context, Poll};
use futures::{future, ready, Future, StreamExt};
use libp2p::swarm::SwarmBuilder;
use libp2p::{PeerId, Swarm};
use parking_lot::RwLock;
use tokio::macros::support::Pin;
use tokio::sync::broadcast::{
    channel as broadcast_channel, Receiver as BroadcastReceiver, Sender as BroadcastSender,
};

use network_interface::network::{Network as NetworkInterface, NetworkEvent};

use crate::behaviour::NimiqBehaviour;
use crate::peer::Peer;

struct SwarmTask {
    swarm: Swarm<NimiqBehaviour>,
    events_tx: BroadcastSender<NetworkEvent<Peer>>,
}

impl SwarmTask {
    fn new(swarm: Swarm<NimiqBehaviour>, events_tx: BroadcastSender<NetworkEvent<Peer>>) -> Self {
        Self { swarm, events_tx }
    }
}

impl Future for SwarmTask {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // The network instance that spawn this task is subscribed to the events channel.
        // If the receiver count drops to zero, the network has gone away and we stop this task.
        if self.events_tx.receiver_count() < 1 {
            return Poll::Ready(());
        }

        // Poll the swarm.
        match ready!(self.swarm.poll_next_unpin(cx)) {
            Some(event) => {
                // Dispatch swarm event on network event broadcast channel.
                if self.events_tx.send(event).is_ok() {
                    // Keep the task alive.
                    Poll::Pending
                } else {
                    // Event dispatch can still fail if the network was dropped after the check above.
                    // Terminate task.
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
    events_tx: BroadcastSender<NetworkEvent<Peer>>,
}

impl Network {
    pub fn new() -> Self {
        let (events_tx, events_rx) = broadcast_channel::<NetworkEvent<Peer>>(4096);

        let peers = Arc::new(RwLock::new(HashMap::new()));
        tokio::spawn(Self::new_network_task(events_rx, &peers));

        let swarm = Self::new_swarm();
        tokio::spawn(SwarmTask::new(swarm, events_tx.clone()));

        Self { peers, events_tx }
    }

    fn new_swarm() -> Swarm<NimiqBehaviour> {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        let local_peer_id = PeerId::from(keypair.public());
        let transport = libp2p::build_tcp_ws_secio_mplex_yamux(keypair).unwrap();
        let behaviour = NimiqBehaviour::new();
        SwarmBuilder::new(transport, behaviour, local_peer_id)
            .incoming_connection_limit(5)
            .outgoing_connection_limit(2)
            .peer_connection_limit(1)
            .build()
    }

    fn new_network_task(
        events_rx: BroadcastReceiver<NetworkEvent<Peer>>,
        peers: &Arc<RwLock<HashMap<PeerId, Arc<Peer>>>>,
    ) -> impl Future<Output = ()> {
        let peers_weak1 = Arc::downgrade(peers);
        let peers_weak2 = Arc::downgrade(peers);
        events_rx
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
                            .remove(peer.id.as_ref())
                            .map(|_| ())
                            .expect("Unknown peer disconnected"),
                    }
                }
                future::ready(())
            })
    }
}

impl NetworkInterface for Network {
    type PeerType = Peer;

    fn get_peers(&self) -> Vec<Arc<Self::PeerType>> {
        self.peers.read().values().cloned().collect()
    }

    fn get_peer(&self, peer_id: &PeerId) -> Option<Arc<Self::PeerType>> {
        self.peers.read().get(peer_id.as_ref()).cloned()
    }

    fn subscribe_events(&self) -> BroadcastReceiver<NetworkEvent<Self::PeerType>> {
        self.events_tx.subscribe()
    }
}

// #[cfg(test)]
// mod tests {
//     use std::sync::Arc;
//     use std::time::Duration;
//
//     use futures::task::{Context, Poll};
//     use futures::{executor, future, FutureExt, StreamExt};
//     use libp2p::core::Multiaddr;
//     use libp2p::Swarm;
//     use parking_lot::RwLock;
//
//     use beserial::{Deserialize, Serialize};
//     use network_interface::message::Message;
//
//     use crate::network::Network;
//
//     #[derive(Deserialize, Serialize)]
//     struct TestMessage {
//         id: u32,
//     }
//     impl Message for TestMessage {
//         const TYPE_ID: u64 = 42;
//     }
//
//     #[tokio::test]
//     async fn test() {
//         let mut net1 = Network::new();
//         let mut net2 = Network::new();
//
//         let addr1 = "/ip4/127.0.0.1/tcp/10001".parse::<Multiaddr>().unwrap();
//         Swarm::listen_on(&mut net1.swarm, addr1.clone()).unwrap();
//
//         Swarm::dial_addr(&mut net2.swarm, addr1).unwrap();
//
//         loop {
//             futures::select! {
//                 e = net1.swarm.next_event().fuse() => println!("[1] Event: {:?}", e),
//                 e = net2.swarm.next_event().fuse() => {
//                     println!("[2] Event: {:?}", e);
//                     break;
//                 }
//             }
//         }
//
//         net1.swarm
//             .send_message(net2.swarm.local_peer_id(), TestMessage { id: 4711 });
//
//         let fut = async move {
//             loop {
//                 futures::select! {
//                     e = net1.swarm.next_event().fuse() => println!("[1] Event: {:?}", e),
//                     e = net2.swarm.next_event().fuse() => println!("[2] Event: {:?}", e)
//                 }
//             }
//         };
//         tokio::time::timeout(Duration::from_secs(3), fut).await;
//     }
// }
