#![allow(dead_code)]

use std::sync::Arc;

use futures::{
    channel::{mpsc, oneshot},
    lock::Mutex as AsyncMutex,
    Stream, FutureExt, SinkExt, StreamExt,
};
use libp2p::core;
use libp2p::core::transport::Boxed;
use libp2p::core::Multiaddr;
use libp2p::core::muxing::StreamMuxerBox;
use libp2p::identity::Keypair;
use libp2p::swarm::{SwarmBuilder, SwarmEvent};
use libp2p::{dns, noise, tcp, websocket, yamux, PeerId, Swarm, Transport};
use tokio::sync::broadcast;
use async_trait::async_trait;
use thiserror::Error;
#[cfg(test)]
use libp2p::core::transport::MemoryTransport;

use beserial::{Serialize, Deserialize};
use nimiq_network_interface::network::{Network as NetworkInterface, NetworkEvent, Topic, ObservablePeerMap};

use crate::{
    behaviour::NimiqBehaviour,
    discovery::{
        behaviour::DiscoveryConfig,
        peer_contacts::PeerContact,
    },
    limit::behaviour::LimitConfig,
    message::behaviour::MessageConfig,
    message::peer::Peer,
};


pub struct Config {
    pub keypair: Keypair,

    pub peer_contact: PeerContact,

    pub discovery: DiscoveryConfig,
    pub message: MessageConfig,
    pub limit: LimitConfig,
}


#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("Dial error: {0}")]
    Dial(#[from] libp2p::swarm::DialError),

    #[error("Failed to send action to swarm task: {0}")]
    Send(#[from] futures::channel::mpsc::SendError),

    #[error("Network action was cancelled: {0}")]
    Canceled(#[from] futures::channel::oneshot::Canceled),
}

type NimiqSwarm = Swarm<NimiqBehaviour>;


#[derive(Debug)]
pub enum NetworkAction {
    Dial {
        peer_id: PeerId,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    DialAddress {
        address: Multiaddr,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
}


pub struct Network {
    local_peer_id: PeerId,
    events_tx: broadcast::Sender<NetworkEvent<Peer>>,
    action_tx: AsyncMutex<mpsc::Sender<NetworkAction>>,
    peers: ObservablePeerMap<Peer>,
}

impl Network {
    /// Create a new libp2p network instance.
    ///
    /// # Arguments
    ///
    ///  - `listen_addr`: The multi-address on which to listen for inbound connections.
    ///  - `config`: The network configuration, containing key pair, and other behaviour-specific configuration.
    ///
    pub fn new(listen_addr: Multiaddr, config: Config) -> Self {
        let swarm = Self::new_swarm(listen_addr, config);
        let peers = swarm.message.peers.clone();

        let local_peer_id = Swarm::local_peer_id(&swarm).clone();

        let (events_tx, _) = broadcast::channel(64);
        let (action_tx, action_rx) = mpsc::channel(64);

        async_std::task::spawn(Self::swarm_task(swarm, events_tx.clone(), action_rx));

        Self {
            local_peer_id,
            events_tx,
            action_tx: AsyncMutex::new(action_tx),
            peers,
        }
    }

    fn new_transport(keypair: &Keypair) -> std::io::Result<Boxed<(PeerId, StreamMuxerBox)>> {
        let transport = {
            let tcp = tcp::TcpConfig::new().nodelay(true);
            let transport = dns::DnsConfig::new(tcp)?;
            let trans_clone = transport.clone();
            let transport = transport.or_transport(websocket::WsConfig::new(trans_clone));

            // Memory transport for testing
            #[cfg(test)]
            let transport = transport.or_transport(MemoryTransport::default());

            transport
        };

        let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
            .into_authentic(keypair)
            .unwrap();

        Ok(transport
            .upgrade(core::upgrade::Version::V1)
            .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
            .multiplex(yamux::YamuxConfig::default())
            .timeout(std::time::Duration::from_secs(20))
            .boxed())
    }

    fn new_swarm(listen_addr: Multiaddr, config: Config) -> Swarm<NimiqBehaviour> {
        let local_peer_id = PeerId::from(config.keypair.clone().public());

        let transport = Self::new_transport(&config.keypair).unwrap();

        let behaviour = NimiqBehaviour::new(config);

        // TODO add proper config
        let mut swarm = SwarmBuilder::new(transport, behaviour, local_peer_id)
            .incoming_connection_limit(5)
            .outgoing_connection_limit(2)
            .peer_connection_limit(1)
            .build();

        Swarm::listen_on(&mut swarm, listen_addr).expect("Failed to listen on provided address");

        swarm
    }

    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    pub async fn dial(&self, peer_id: PeerId) -> Result<(), NetworkError> {
        let (output_tx, output_rx) = oneshot::channel();
        self.action_tx.lock().await.send(NetworkAction::Dial {
            peer_id,
            output: output_tx,
        }).await?;
        output_rx.await?
    }

    pub async fn dial_address(&self, address: Multiaddr) -> Result<(), NetworkError> {
        let (output_tx, output_rx) = oneshot::channel();
        self.action_tx.lock().await.send(NetworkAction::DialAddress {
            address,
            output: output_tx,
        }).await?;
        output_rx.await?
    }

    async fn swarm_task(
        mut swarm: NimiqSwarm,
        events_tx: broadcast::Sender<NetworkEvent<Peer>>,
        mut action_rx: mpsc::Receiver<NetworkAction>,
    ) {
        // TODO: Use swarm events for

        loop {
            futures::select! {
                event = swarm.next_event().fuse() => {
                    match event {
                       SwarmEvent::Behaviour(event) => {
                            log::debug!("Swarm task received event: {:?}", event);

                            if let Err(event) = events_tx.send(event) {
                                log::error!("Failed to notify subscribers about network event: {:?}", event);
                            }
                        },
                        _ => {},
                    }
                },
                action_opt = action_rx.next().fuse() => {
                    if let Some(action) = action_opt {
                        Self::perform_action(action, &mut swarm).await.unwrap();
                    }
                    else {
                        // `action_rx.next()` will return `None` if all senders (i.e. the `Network` object) are dropped.
                        break;
                    }
                },
            };
        }
    }

    async fn perform_action(action: NetworkAction, swarm: &mut NimiqSwarm) -> Result<(), NetworkError> {
        log::debug!("Swarm task: performing action: {:?}", action);

        match action {
            NetworkAction::Dial { peer_id, output } => {
                output.send(Swarm::dial(swarm, &peer_id).map_err(Into::into)).ok();
            },
            NetworkAction::DialAddress { address, output } => {
                output.send(Swarm::dial_addr(swarm, address)
                    .map_err(|l| NetworkError::Dial(libp2p::swarm::DialError::ConnectionLimit(l)))).ok();
            },
        }

        Ok(())
    }
}

#[async_trait]
impl NetworkInterface for Network {
    type PeerType = Peer;
    type Error = NetworkError;

    fn get_peer_updates(&self) -> (Vec<Arc<Self::PeerType>>, broadcast::Receiver<NetworkEvent<Self::PeerType>>) {
        self.peers.subscribe()
    }

    fn get_peers(&self) -> Vec<Arc<Self::PeerType>> {
        self.peers.get_peers()
    }

    fn get_peer(&self, peer_id: PeerId) -> Option<Arc<Self::PeerType>> {
        self.peers.get_peer(&peer_id)
    }

    fn subscribe_events(&self) -> broadcast::Receiver<NetworkEvent<Self::PeerType>> {
        self.events_tx.subscribe()
    }

    async fn subscribe<T>(_topic: &T) -> Box<dyn Stream<Item = (T::Item, Self::PeerType)> + Send>
        where
            T: Topic + Sync,
    {
        unimplemented!()
    }

    async fn publish<T>(_topic: &T, _item: <T as Topic>::Item)
        where
            T: Topic + Sync,
    {
        unimplemented!()
    }

    async fn dht_get<K, V>(&self, _k: &K) -> Result<V, Self::Error>
        where
            K: AsRef<[u8]> + Send + Sync,
            V: Deserialize + Send + Sync,
    {
        unimplemented!()
    }

    async fn dht_put<K, V>(&self, _k: &K, _v: &V) -> Result<(), Self::Error>
        where
            K: AsRef<[u8]> + Send + Sync,
            V: Serialize + Send + Sync,
    {
        unimplemented!()
    }
}


#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::StreamExt;
    use libp2p::{
        multiaddr::{multiaddr, Multiaddr},
        identity::Keypair,
        swarm::KeepAlive,
        PeerId,
    };
    use rand::{thread_rng, Rng};

    use beserial::{Deserialize, Serialize};
    use nimiq_network_interface::{
        message::Message,
        network::{Network as NetworkInterface},
        peer::{Peer as PeerInterface, CloseReason},
    };

    use crate::{
        discovery::{
            behaviour::DiscoveryConfig,
            peer_contacts::{PeerContact, Protocols, Services},
        },
        message::{
            behaviour::MessageConfig,
            peer::Peer,
        },
    };
    use super::{Network, Config};
    use nimiq_network_interface::network::NetworkEvent;

    #[derive(Clone, Debug, Deserialize, Serialize)]
    struct TestMessage {
        id: u32,
    }

    impl Message for TestMessage {
        const TYPE_ID: u64 = 42;
    }

    fn network_config(address: Multiaddr) -> Config {
        let keypair = Keypair::generate_ed25519();

        let mut peer_contact = PeerContact {
            addresses: vec![address],
            public_key: keypair.public().clone(),
            services: Services::all(),
            timestamp: None,
        };
        peer_contact.set_current_time();

        Config {
            keypair,
            peer_contact,
            discovery: DiscoveryConfig {
                genesis_hash: Default::default(),
                update_interval: Duration::from_secs(60),
                min_recv_update_interval: Duration::from_secs(30),
                update_limit: 64,
                protocols_filter: Protocols::all(),
                services_filter: Services::all(),
                min_send_update_interval: Duration::from_secs(30),
                house_keeping_interval: Duration::from_secs(60),
                keep_alive: KeepAlive::Yes,
            },
            message: MessageConfig::default(),
            limit: Default::default(),
        }
    }

    fn assert_peer_joined(event: &NetworkEvent<Peer>, peer_id: &PeerId) {
        if let NetworkEvent::PeerJoined(peer) = event {
            assert_eq!(&peer.id, peer_id);
        }
        else {
            panic!("Event is not a NetworkEvent::PeerJoined: {:?}", event);
        }
    }

    async fn create_connected_networks() -> (Network, Network) {
        log::info!("Creating connected test networks:");
        let addr1 = multiaddr![Memory(thread_rng().gen::<u64>())];
        let addr2 = multiaddr![Memory(thread_rng().gen::<u64>())];

        let net1 = Network::new(addr1.clone(), network_config(addr1.clone()));
        let net2 = Network::new(addr2.clone(), network_config(addr2.clone()));

        log::info!("Network 1: address={}, peer_id={}", addr1, net1.local_peer_id);
        log::info!("Network 2: address={}, peer_id={}", addr2, net2.local_peer_id);

        log::info!("Dialing peer 1 from peer 2...");
        net2.dial_address(addr1).await.unwrap();

        let mut events1 = net1.subscribe_events();
        let mut events2 = net2.subscribe_events();

        log::info!("Waiting for events");

        let event1 = events1.next().await.unwrap().unwrap();
        log::debug!("event1 = {:?}", event1);
        assert_peer_joined(&event1, &net2.local_peer_id);

        let event2 = events2.next().await.unwrap().unwrap();
        log::debug!("event2 = {:?}", event2);
        assert_peer_joined(&event2, &net1.local_peer_id);

        (net1, net2)
    }

    #[tokio::test]
    async fn two_networks_can_connect() {
        let (net1, net2) = create_connected_networks().await;
        assert_eq!(net1.get_peers().len(), 1);
        assert_eq!(net2.get_peers().len(), 1);

        let peer2 = net1.get_peer(net2.local_peer_id().clone()).unwrap();
        let peer1 = net2.get_peer(net1.local_peer_id().clone()).unwrap();
        assert_eq!(peer2.id(), net2.local_peer_id);
        assert_eq!(peer1.id(), net1.local_peer_id);

        log::info!("Test finished");
    }

    #[tokio::test]
    async fn one_peer_can_talk_to_another() {
        let (net1, net2) = create_connected_networks().await;

        let peer2 = net1.get_peer(net2.local_peer_id().clone()).unwrap();
        let peer1 = net2.get_peer(net1.local_peer_id().clone()).unwrap();

        let mut msgs = peer1.receive::<TestMessage>();

        peer2.send(&TestMessage { id: 4711 }).await.unwrap();

        log::info!("Send complete");

        let msg = msgs.next().await.unwrap();

        assert_eq!(msg.id, 4711);
    }

    #[tokio::test]
    async fn both_peers_can_talk_with_each_other() {
        let (net1, net2) = create_connected_networks().await;

        let peer2 = net1.get_peer(net2.local_peer_id().clone()).unwrap();
        let peer1 = net2.get_peer(net1.local_peer_id().clone()).unwrap();

        let mut in1 = peer1.receive::<TestMessage>();
        let mut in2 = peer2.receive::<TestMessage>();

        peer1.send(&TestMessage { id: 1337 }).await.unwrap();
        peer2.send(&TestMessage { id: 420 }).await.unwrap();

        let msg1 = in2.next().await.unwrap();
        let msg2 = in1.next().await.unwrap();

        assert_eq!(msg1.id, 1337);
        assert_eq!(msg2.id, 420);
    }

    fn assert_peer_left(event: &NetworkEvent<Peer>, peer_id: &PeerId) {
        if let NetworkEvent::PeerLeft(peer) = event {
            assert_eq!(&peer.id, peer_id);
        }
        else {
            panic!("Event is not a NetworkEvent::PeerLeft: {:?}", event);
        }
    }

    #[tokio::test]
    async fn connections_are_properly_closed() {
        let (net1, net2) = create_connected_networks().await;

        let peer2 = net1.get_peer(net2.local_peer_id().clone()).unwrap();
        peer2.close(CloseReason::Other).await;

        let mut events1 = net1.subscribe_events();
        let mut events2 = net2.subscribe_events();

        let event2 = events2.next().await.unwrap().unwrap();
        assert_peer_left(&event2, net1.local_peer_id());
        log::debug!("event2 = {:?}", event2);

        let event1 = events1.next().await.unwrap().unwrap();
        assert_peer_left(&event1, net2.local_peer_id());
        log::debug!("event1 = {:?}", event1);

        assert_eq!(net1.get_peers().len(), 0);
        assert_eq!(net2.get_peers().len(), 0);
    }
}
