#![allow(dead_code)]

use std::{collections::HashMap, pin::Pin, sync::Arc};

use async_trait::async_trait;
use bytes::{buf::BufExt, Bytes};
use futures::{
    channel::{mpsc, oneshot},
    future::FutureExt,
    lock::Mutex as AsyncMutex,
    sink::SinkExt,
    stream::{BoxStream, Stream, StreamExt},
};
use libp2p::{
    core,
    core::{
        connection::ConnectionLimits, muxing::StreamMuxerBox, network::NetworkInfo,
        transport::Boxed,
    },
    dns,
    gossipsub::{
        GossipsubConfig, GossipsubConfigBuilder, GossipsubEvent, GossipsubMessage, IdentTopic,
        MessageAcceptance, MessageId, TopicHash,
    },
    identify::IdentifyEvent,
    identity::Keypair,
    kad::{GetRecordOk, KademliaConfig, KademliaEvent, QueryId, QueryResult, Quorum, Record},
    noise,
    swarm::{NetworkBehaviourAction, NotifyHandler, SwarmBuilder, SwarmEvent},
    tcp, websocket, yamux, Multiaddr, PeerId, Swarm, Transport,
};
use thiserror::Error;
use tokio::sync::broadcast;
use tracing::Instrument;

#[cfg(test)]
use libp2p::core::transport::MemoryTransport;

use beserial::{Deserialize, Serialize};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    message::{Message, MessageType},
    network::{Network as NetworkInterface, NetworkEvent, PubsubId, Topic},
    peer::Peer as PeerInterface,
    peer_map::ObservablePeerMap,
};
use nimiq_utils::time::OffsetTime;

use crate::{
    behaviour::{NimiqBehaviour, NimiqEvent, NimiqNetworkBehaviourError},
    discovery::{behaviour::DiscoveryConfig, handler::HandlerInEvent, peer_contacts::PeerContact},
    limit::behaviour::LimitConfig,
    message::behaviour::MessageConfig,
    message::peer::Peer,
};

/// Maximum simultaneous libp2p connections per peer
const MAX_CONNECTIONS_PER_PEER: u32 = 1;

pub struct Config {
    pub keypair: Keypair,

    pub peer_contact: PeerContact,

    pub min_peers: usize,

    pub discovery: DiscoveryConfig,
    pub message: MessageConfig,
    pub limit: LimitConfig,
    pub kademlia: KademliaConfig,
    pub gossipsub: GossipsubConfig,
}

impl Config {
    pub fn new(keypair: Keypair, peer_contact: PeerContact, genesis_hash: Blake2bHash) -> Self {
        // Hardcoding the minimum number of peers in mesh network before adding more
        // TODO: Maybe change this to a mesh limits configuration argument of this function
        let gossipsub_config = GossipsubConfigBuilder::default()
            .mesh_n_low(3)
            .validate_messages()
            .build()
            .expect("Invalid Gossipsub config");

        Self {
            keypair,
            peer_contact,
            discovery: DiscoveryConfig::new(genesis_hash),
            message: MessageConfig::default(),
            limit: LimitConfig::default(),
            kademlia: KademliaConfig::default(),
            gossipsub: gossipsub_config,
            min_peers: 5,
        }
    }
}

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("Dial error: {0}")]
    Dial(#[from] libp2p::swarm::DialError),

    #[error("Failed to send action to swarm task: {0}")]
    Send(#[from] futures::channel::mpsc::SendError),

    #[error("Network action was cancelled: {0}")]
    Canceled(#[from] futures::channel::oneshot::Canceled),

    #[error("Serialization error: {0}")]
    Serialization(#[from] beserial::SerializingError),

    #[error("Network behaviour error: {0}")]
    Behaviour(#[from] NimiqNetworkBehaviourError),

    #[error("DHT store error: {0:?}")]
    DhtStore(libp2p::kad::store::Error),

    #[error("DHT GetRecord error: {0:?}")]
    DhtGetRecord(libp2p::kad::GetRecordError),

    #[error("DHT PutRecord error: {0:?}")]
    DhtPutRecord(libp2p::kad::PutRecordError),

    #[error("Gossipsub Publish error: {0:?}")]
    GossipsubPublish(libp2p::gossipsub::error::PublishError),

    #[error("Gossipsub Subscription error: {0:?}")]
    GossipsubSubscription(libp2p::gossipsub::error::SubscriptionError),

    #[error("Already subscribed to topic: {topic_name}")]
    AlreadySubscribed { topic_name: String },
}

impl From<libp2p::kad::store::Error> for NetworkError {
    fn from(e: libp2p::kad::store::Error) -> Self {
        Self::DhtStore(e)
    }
}

impl From<libp2p::kad::GetRecordError> for NetworkError {
    fn from(e: libp2p::kad::GetRecordError) -> Self {
        Self::DhtGetRecord(e)
    }
}

impl From<libp2p::kad::PutRecordError> for NetworkError {
    fn from(e: libp2p::kad::PutRecordError) -> Self {
        Self::DhtPutRecord(e)
    }
}

impl From<libp2p::gossipsub::error::PublishError> for NetworkError {
    fn from(e: libp2p::gossipsub::error::PublishError) -> Self {
        Self::GossipsubPublish(e)
    }
}

impl From<libp2p::gossipsub::error::SubscriptionError> for NetworkError {
    fn from(e: libp2p::gossipsub::error::SubscriptionError) -> Self {
        Self::GossipsubSubscription(e)
    }
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
    DhtGet {
        key: Vec<u8>,
        output: oneshot::Sender<Result<Option<Vec<u8>>, NetworkError>>,
    },
    DhtPut {
        key: Vec<u8>,
        value: Vec<u8>,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    Subscribe {
        topic_name: String,
        validate: bool,
        output: oneshot::Sender<
            Result<mpsc::Receiver<(GossipsubMessage, MessageId, PeerId)>, NetworkError>,
        >,
    },
    Publish {
        topic_name: String,
        data: Vec<u8>,
        output: oneshot::Sender<Result<MessageId, NetworkError>>,
    },
    NetworkInfo {
        output: oneshot::Sender<NetworkInfo>,
    },
    Validate {
        message_id: MessageId,
        source: PeerId,
        output: oneshot::Sender<Result<bool, NetworkError>>,
    },
    ReceiveFromAll {
        type_id: MessageType,
        output: mpsc::Sender<(Bytes, Arc<Peer>)>,
    },
}

struct TaskState {
    dht_puts: HashMap<QueryId, oneshot::Sender<Result<(), NetworkError>>>,
    dht_gets: HashMap<QueryId, oneshot::Sender<Result<Option<Vec<u8>>, NetworkError>>>,
    gossip_topics: HashMap<TopicHash, (mpsc::Sender<(GossipsubMessage, MessageId, PeerId)>, bool)>,
    connected_tx: Option<oneshot::Sender<()>>,
    incoming_listeners: HashMap<Multiaddr, Multiaddr>,
}

impl TaskState {
    pub fn new(connected_tx: oneshot::Sender<()>) -> Self {
        Self {
            dht_puts: HashMap::new(),
            dht_gets: HashMap::new(),
            gossip_topics: HashMap::new(),
            connected_tx: Some(connected_tx),
            incoming_listeners: HashMap::new(),
        }
    }

    fn is_connected(&self) -> bool {
        self.connected_tx.is_none()
    }

    fn set_connected(&mut self) {
        if let Some(connected_tx) = self.connected_tx.take() {
            connected_tx.send(()).ok();
        }
    }
}

#[derive(Debug)]
pub struct GossipsubId<P> {
    message_id: MessageId,
    propagation_source: P,
}

impl PubsubId<PeerId> for GossipsubId<PeerId> {
    fn propagation_source(&self) -> PeerId {
        self.propagation_source
    }
}

pub struct Network {
    local_peer_id: PeerId,
    events_tx: broadcast::Sender<NetworkEvent<Peer>>,
    action_tx: mpsc::Sender<NetworkAction>,
    peers: ObservablePeerMap<Peer>,
    connected_rx: AsyncMutex<Option<oneshot::Receiver<()>>>,
}

impl Network {
    /// Create a new libp2p network instance.
    ///
    /// # Arguments
    ///
    ///  - `listen_addresses`: The multi-addresses on which to listen for inbound connections.
    ///  - `clock`: The clock that is used to establish the network time. The discovery behaviour will determine the
    ///             offset by exchanging their wall-time with other peers.
    ///  - `config`: The network configuration, containing key pair, and other behaviour-specific configuration.
    ///
    pub async fn new(
        listen_addresses: Vec<Multiaddr>,
        clock: Arc<OffsetTime>,
        config: Config,
    ) -> Self {
        let min_peers = config.min_peers;

        let swarm = Self::new_swarm(listen_addresses, clock, config);
        let peers = swarm.message.peers.clone();

        let local_peer_id = *Swarm::local_peer_id(&swarm);

        let (events_tx, _) = broadcast::channel(64);
        let (action_tx, action_rx) = mpsc::channel(64);

        let (connected_tx, connected_rx) = oneshot::channel();

        async_std::task::spawn(Self::swarm_task(
            swarm,
            events_tx.clone(),
            action_rx,
            connected_tx,
            min_peers,
        ));

        Self {
            local_peer_id,
            events_tx,
            action_tx,
            peers,
            connected_rx: AsyncMutex::new(Some(connected_rx)),
        }
    }

    pub async fn wait_connected(&self) {
        tracing::info!("Waiting for peers to connect");
        if let Some(connected_rx) = self.connected_rx.lock().await.take() {
            connected_rx.await.unwrap()
        }
    }

    fn new_transport(keypair: &Keypair) -> std::io::Result<Boxed<(PeerId, StreamMuxerBox)>> {
        let transport = {
            // Websocket over TCP/DNS
            let transport =
                websocket::WsConfig::new(dns::DnsConfig::new(tcp::TcpConfig::new().nodelay(true))?);

            // Memory transport for testing
            // TODO: Use websocket over the memory transport
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

    fn new_swarm(
        listen_addresses: Vec<Multiaddr>,
        clock: Arc<OffsetTime>,
        config: Config,
    ) -> Swarm<NimiqBehaviour> {
        let local_peer_id = PeerId::from(config.keypair.public());

        let transport = Self::new_transport(&config.keypair).unwrap();

        let behaviour = NimiqBehaviour::new(config, clock);

        let limits = ConnectionLimits::default()
            .with_max_pending_incoming(Some(5))
            .with_max_pending_outgoing(Some(2))
            .with_max_established_incoming(Some(4800))
            .with_max_established_outgoing(Some(4800))
            .with_max_established_per_peer(Some(MAX_CONNECTIONS_PER_PEER));

        // TODO add proper config
        let mut swarm = SwarmBuilder::new(transport, behaviour, local_peer_id)
            .connection_limits(limits)
            .build();

        for listen_addr in listen_addresses {
            Swarm::listen_on(&mut swarm, listen_addr)
                .expect("Failed to listen on provided address");
        }

        swarm
    }

    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    async fn swarm_task(
        mut swarm: NimiqSwarm,
        events_tx: broadcast::Sender<NetworkEvent<Peer>>,
        mut action_rx: mpsc::Receiver<NetworkAction>,
        connected_tx: oneshot::Sender<()>,
        min_peers: usize,
    ) {
        let mut task_state = TaskState::new(connected_tx);

        let peer_id = Swarm::local_peer_id(&swarm);
        let task_span = tracing::debug_span!("swarm task", peer_id=?peer_id);

        async move {
            loop {
                futures::select! {
                    event = swarm.next_event().fuse() => {
                        tracing::debug!(event=?event, "swarm task received event");
                        Self::handle_event(event, &events_tx, &mut swarm, &mut task_state, min_peers).await;
                    },
                    action_opt = action_rx.next().fuse() => {
                        if let Some(action) = action_opt {
                            Self::perform_action(action, &mut swarm, &mut task_state).await.unwrap();
                        }
                        else {
                            // `action_rx.next()` will return `None` if all senders (i.e. the `Network` object) are dropped.
                            break;
                        }
                    },
                };
            }
        }
            .instrument(task_span)
            .await
    }

    async fn handle_event(
        event: SwarmEvent<NimiqEvent, NimiqNetworkBehaviourError>,
        events_tx: &broadcast::Sender<NetworkEvent<Peer>>,
        swarm: &mut NimiqSwarm,
        state: &mut TaskState,
        min_peers: usize,
    ) {
        match event {
            SwarmEvent::ConnectionEstablished {
                peer_id,
                endpoint,
                num_established,
            } => {
                if let Some(listen_addr) = state
                    .incoming_listeners
                    .get(&endpoint.get_remote_address().clone())
                {
                    tracing::debug!(
                        "Adding peer {:?} listen address to the peer contact book: {:?}",
                        peer_id,
                        listen_addr
                    );
                    swarm.kademlia.add_address(&peer_id, listen_addr.clone());

                    // TODO: Rework peer address book handling
                    swarm
                        .discovery
                        .events
                        .push_back(NetworkBehaviourAction::NotifyHandler {
                            peer_id,
                            handler: NotifyHandler::Any,
                            event: HandlerInEvent::ObservedAddress(vec![listen_addr.clone()]),
                        });

                    state
                        .incoming_listeners
                        .remove(&endpoint.get_remote_address().clone());
                }

                if !state.is_connected() {
                    tracing::debug!(
                        num_established,
                        min_peers,
                        "connected to {} peers (waiting for {})",
                        num_established,
                        min_peers
                    );

                    if num_established.get() as usize >= min_peers {
                        state.set_connected();

                        // Bootstrap Kademlia
                        tracing::debug!("Bootstrapping DHT");
                        if swarm.kademlia.bootstrap().is_err() {
                            tracing::error!("Bootstrapping DHT error: No known peers");
                        }
                    }
                }
            }

            SwarmEvent::IncomingConnection {
                local_addr,
                send_back_addr,
            } => {
                tracing::trace!(
                    "Incoming connection from address {:?}, listen address: {:?}",
                    send_back_addr,
                    local_addr
                );
                state.incoming_listeners.insert(send_back_addr, local_addr);
            }

            SwarmEvent::IncomingConnectionError {
                local_addr: _,
                send_back_addr,
                error,
            } => {
                tracing::warn!("Incoming connection error: {:?}", error);
                state.incoming_listeners.remove(&send_back_addr);
            }

            //SwarmEvent::ConnectionClosed { .. } => {},
            SwarmEvent::Behaviour(event) => {
                match event {
                    NimiqEvent::Message(event) => {
                        tracing::trace!(event = ?event, "network event");
                        events_tx.send(event).ok();
                    }
                    NimiqEvent::Dht(event) => {
                        match event {
                            KademliaEvent::QueryResult { id, result, .. } => {
                                match result {
                                    QueryResult::GetRecord(result) => {
                                        if let Some(output) = state.dht_gets.remove(&id) {
                                            let result = result.map_err(Into::into).and_then(
                                                |GetRecordOk { mut records }| {
                                                    // TODO: What do we do, if we get multiple records?
                                                    let data_opt =
                                                        records.pop().map(|r| r.record.value);
                                                    Ok(data_opt)
                                                },
                                            );
                                            output.send(result).ok();
                                        } else {
                                            tracing::warn!(query_id = ?id, "GetRecord query result for unknown query ID");
                                        }
                                    }
                                    QueryResult::PutRecord(result) => {
                                        // dht_put resolved
                                        if let Some(output) = state.dht_puts.remove(&id) {
                                            output
                                                .send(result.map(|_| ()).map_err(Into::into))
                                                .ok();
                                        } else {
                                            tracing::warn!(query_id = ?id, "PutRecord query result for unknown query ID");
                                        }
                                    }
                                    QueryResult::Bootstrap(result) => match result {
                                        Ok(result) => {
                                            tracing::debug!(result = ?result, "DHT bootstrap successful")
                                        }
                                        Err(e) => tracing::error!("DHT bootstrap error: {:?}", e),
                                    },
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    NimiqEvent::Gossip(event) => match event {
                        GossipsubEvent::Message {
                            propagation_source,
                            message_id,
                            message,
                        } => {
                            tracing::debug!(id = ?message_id, source = ?propagation_source, message = ?message, "received message");

                            if let Some(topic_info) = state.gossip_topics.get_mut(&message.topic) {
                                let (output, validate) = topic_info;
                                if !&*validate {
                                    swarm
                                        .gossipsub
                                        .report_message_validation_result(
                                            &message_id,
                                            &propagation_source,
                                            MessageAcceptance::Accept,
                                        )
                                        .ok();
                                }
                                output
                                    .send((message, message_id, propagation_source))
                                    .await
                                    .ok();
                            } else {
                                tracing::warn!(topic = ?message.topic, "unknown topic hash");
                            }
                        }
                        GossipsubEvent::Subscribed { peer_id, topic } => {
                            tracing::debug!(peer_id = ?peer_id, topic = ?topic, "peer subscribed to topic");
                        }
                        GossipsubEvent::Unsubscribed { peer_id, topic } => {
                            tracing::debug!(peer_id = ?peer_id, topic = ?topic, "peer unsubscribed");
                        }
                    },
                    NimiqEvent::Identify(event) => {
                        match event {
                            IdentifyEvent::Received {
                                peer_id,
                                info,
                                observed_addr,
                            } => {
                                tracing::debug!("Received identifying info from peer {:?} at address {:?}: {:?}", peer_id, observed_addr, info);
                                for listen_addr in info.listen_addrs.clone() {
                                    swarm.kademlia.add_address(&peer_id, listen_addr);
                                }

                                // TODO: Rework peer address book handling
                                swarm.discovery.events.push_back(
                                    NetworkBehaviourAction::NotifyHandler {
                                        peer_id,
                                        handler: NotifyHandler::Any,
                                        event: HandlerInEvent::ObservedAddress(info.listen_addrs),
                                    },
                                );
                            }
                            IdentifyEvent::Sent { peer_id } => {
                                tracing::debug!("Sent identifiyng info to peer {:?}", peer_id);
                            }
                            IdentifyEvent::Error { peer_id, error } => {
                                tracing::error!(
                                    "Error while identifying remote peer {:?}: {:?}",
                                    peer_id,
                                    error
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    async fn perform_action(
        action: NetworkAction,
        swarm: &mut NimiqSwarm,
        state: &mut TaskState,
    ) -> Result<(), NetworkError> {
        tracing::debug!(action = ?action, "performing action");

        match action {
            NetworkAction::Dial { peer_id, output } => {
                output
                    .send(Swarm::dial(swarm, &peer_id).map_err(Into::into))
                    .ok();
            }
            NetworkAction::DialAddress { address, output } => {
                output
                    .send(Swarm::dial_addr(swarm, address).map_err(|l| {
                        NetworkError::Dial(libp2p::swarm::DialError::ConnectionLimit(l))
                    }))
                    .ok();
            }
            NetworkAction::DhtGet { key, output } => {
                let query_id = swarm.kademlia.get_record(&key.into(), Quorum::One);
                state.dht_gets.insert(query_id, output);
            }
            NetworkAction::DhtPut { key, value, output } => {
                let local_peer_id = Swarm::local_peer_id(&swarm);

                let record = Record {
                    key: key.into(),
                    value,
                    publisher: Some(*local_peer_id),
                    expires: None, // TODO: Records should expire at some point in time
                };

                match swarm.kademlia.put_record(record, Quorum::One) {
                    Ok(query_id) => {
                        // Remember put operation to resolve when we receive a `QueryResult::PutRecord`
                        state.dht_puts.insert(query_id, output);
                    }
                    Err(e) => {
                        output.send(Err(e.into())).ok();
                    }
                }
            }
            NetworkAction::Subscribe {
                topic_name,
                validate,
                output,
            } => {
                let topic = IdentTopic::new(topic_name.clone());

                match swarm.gossipsub.subscribe(&topic) {
                    // New subscription. Insert the sender into our subscription table.
                    Ok(true) => {
                        let (tx, rx) = mpsc::channel(16);

                        state.gossip_topics.insert(topic.hash(), (tx, validate));

                        output.send(Ok(rx)).ok();
                    }

                    // Apparently we're already subscribed.
                    Ok(false) => {
                        output
                            .send(Err(NetworkError::AlreadySubscribed { topic_name }))
                            .ok();
                    }

                    // Failed. Send back error.
                    Err(e) => {
                        output.send(Err(e.into())).ok();
                    }
                }
            }
            NetworkAction::Publish {
                topic_name,
                data,
                output,
            } => {
                // TODO: Check if we're subscribed to the topic, otherwise we can't publish
                let topic = IdentTopic::new(topic_name);
                output
                    .send(swarm.gossipsub.publish(topic, data).map_err(Into::into))
                    .ok();
            }
            NetworkAction::NetworkInfo { output } => {
                output.send(Swarm::network_info(swarm)).ok();
            }
            NetworkAction::Validate {
                message_id,
                source,
                output,
            } => {
                output
                    .send(Ok(swarm.gossipsub.report_message_validation_result(
                        &message_id,
                        &source,
                        MessageAcceptance::Accept,
                    )?))
                    .ok();
            }
            NetworkAction::ReceiveFromAll { type_id, output } => {
                swarm.message.receive_from_all(type_id, output);
            }
        }

        Ok(())
    }

    pub async fn network_info(&self) -> Result<NetworkInfo, NetworkError> {
        let (output_tx, output_rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::NetworkInfo { output: output_tx })
            .await?;
        Ok(output_rx.await?)
    }
}

#[async_trait]
impl NetworkInterface for Network {
    type PeerType = Peer;
    type AddressType = Multiaddr;
    type Error = NetworkError;
    type PubsubId = GossipsubId<PeerId>;

    fn get_peer_updates(
        &self,
    ) -> (
        Vec<Arc<Self::PeerType>>,
        broadcast::Receiver<NetworkEvent<Self::PeerType>>,
    ) {
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

    /// Implements `receive_from_all`, but instead of selecting over all peer message streams, we register a channel in
    /// the network. The sender is copied to new peers when they're instantiated.
    fn receive_from_all<'a, T: Message>(&self) -> BoxStream<'a, (T, Arc<Peer>)> {
        let mut action_tx = self.action_tx.clone();

        // Future to register the channel.
        let register_stream = async move {
            let (tx, rx) = mpsc::channel(0);

            action_tx
                .send(NetworkAction::ReceiveFromAll {
                    type_id: T::TYPE_ID.into(),
                    output: tx,
                })
                .await
                .expect("Sending action to network task failed.");

            rx
        };

        register_stream
            .flatten_stream()
            .filter_map(|(data, peer)| async move {
                // Map the (data, peer) stream to (message, peer) by deserializing the messages.
                match <T as Deserialize>::deserialize(&mut data.reader()) {
                    Ok(message) => Some((message, peer)),
                    Err(e) => {
                        tracing::error!("Failed to deserialize message: {}", e);
                        None
                    }
                }
            })
            .boxed()
    }

    async fn subscribe<T>(
        &self,
        topic: &T,
    ) -> Result<Pin<Box<dyn Stream<Item = (T::Item, Self::PubsubId)> + Send>>, Self::Error>
    where
        T: Topic + Sync,
    {
        let (tx, rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::Subscribe {
                topic_name: topic.topic(),
                validate: topic.validate(),
                output: tx,
            })
            .await?;

        // Receive the mpsc::Receiver, but propagate errors first.
        let rx = rx.await??;

        Ok(rx
            .map(|(msg, msg_id, source)| {
                let item: <T as Topic>::Item =
                    Deserialize::deserialize_from_vec(&msg.data).unwrap();
                let id = GossipsubId {
                    message_id: msg_id,
                    propagation_source: source,
                };
                (item, id)
            })
            .boxed())
    }

    async fn publish<T>(&self, topic: &T, item: <T as Topic>::Item) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let (output_tx, output_rx) = oneshot::channel();

        let mut buf = vec![];
        item.serialize(&mut buf)?;

        self.action_tx
            .clone()
            .send(NetworkAction::Publish {
                topic_name: topic.topic(),
                data: buf,
                output: output_tx,
            })
            .await?;

        let _message_id = output_rx.await??;

        Ok(())
    }

    async fn validate_message(&self, id: Self::PubsubId) -> Result<bool, Self::Error> {
        let (output_tx, output_rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::Validate {
                message_id: id.message_id,
                source: id.propagation_source,
                output: output_tx,
            })
            .await?;

        output_rx.await?
    }

    async fn dht_get<K, V>(&self, k: &K) -> Result<Option<V>, Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Deserialize + Send + Sync,
    {
        let (output_tx, output_rx) = oneshot::channel();
        self.action_tx
            .clone()
            .send(NetworkAction::DhtGet {
                key: k.as_ref().to_owned(),
                output: output_tx,
            })
            .await?;

        if let Some(data) = output_rx.await?? {
            Ok(Some(Deserialize::deserialize_from_vec(&data)?))
        } else {
            Ok(None)
        }
    }

    async fn dht_put<K, V>(&self, k: &K, v: &V) -> Result<(), Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Serialize + Send + Sync,
    {
        let (output_tx, output_rx) = oneshot::channel();

        let mut buf = vec![];
        v.serialize(&mut buf)?;

        self.action_tx
            .clone()
            .send(NetworkAction::DhtPut {
                key: k.as_ref().to_owned(),
                value: buf,
                output: output_tx,
            })
            .await?;
        output_rx.await?
    }

    async fn dial_peer(&self, peer_id: PeerId) -> Result<(), NetworkError> {
        let (output_tx, output_rx) = oneshot::channel();
        self.action_tx
            .clone()
            .send(NetworkAction::Dial {
                peer_id,
                output: output_tx,
            })
            .await?;
        output_rx.await?
    }

    async fn dial_address(&self, address: Multiaddr) -> Result<(), NetworkError> {
        let (output_tx, output_rx) = oneshot::channel();
        self.action_tx
            .clone()
            .send(NetworkAction::DialAddress {
                address,
                output: output_tx,
            })
            .await?;
        output_rx.await?
    }

    fn get_local_peer_id(&self) -> <Self::PeerType as PeerInterface>::Id {
        self.local_peer_id
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use futures::{Stream, StreamExt};
    use libp2p::{
        gossipsub::GossipsubConfigBuilder,
        identity::Keypair,
        multiaddr::{multiaddr, Multiaddr},
        swarm::KeepAlive,
        PeerId,
    };
    use rand::{thread_rng, Rng};

    use beserial::{Deserialize, Serialize};
    use nimiq_network_interface::{
        message::Message,
        network::Network as NetworkInterface,
        peer::{CloseReason, Peer as PeerInterface},
    };
    use nimiq_utils::time::OffsetTime;

    use super::{Config, Network};
    use crate::{
        discovery::{
            behaviour::DiscoveryConfig,
            peer_contacts::{PeerContact, Protocols, Services},
        },
        message::peer::Peer,
    };
    use nimiq_network_interface::network::{NetworkEvent, Topic};

    #[derive(Clone, Debug, Deserialize, Serialize)]
    struct TestMessage {
        id: u32,
    }

    impl Message for TestMessage {
        const TYPE_ID: u64 = 42;
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    struct TestMessage2 {
        #[beserial(len_type(u8))]
        x: String,
    }

    impl Message for TestMessage2 {
        const TYPE_ID: u64 = 43;
    }

    fn network_config(address: Multiaddr) -> Config {
        let keypair = Keypair::generate_ed25519();

        let mut peer_contact = PeerContact {
            addresses: vec![address],
            public_key: keypair.public(),
            services: Services::all(),
            timestamp: None,
        };
        peer_contact.set_current_time();

        let gossipsub = GossipsubConfigBuilder::default()
            .build()
            .expect("Invalid Gossipsub config");

        Config {
            keypair,
            peer_contact,
            min_peers: 0,
            discovery: DiscoveryConfig {
                genesis_hash: Default::default(),
                update_interval: Duration::from_secs(60),
                min_recv_update_interval: Duration::from_secs(30),
                update_limit: 64,
                protocols_filter: Protocols::all(),
                services_filter: Services::all(),
                min_send_update_interval: Duration::from_secs(30),
                house_keeping_interval: Duration::from_secs(60),
                keep_alive: KeepAlive::No,
            },
            message: Default::default(),
            limit: Default::default(),
            kademlia: Default::default(),
            gossipsub,
        }
    }

    fn assert_peer_joined(event: &NetworkEvent<Peer>, peer_id: &PeerId) {
        if let NetworkEvent::PeerJoined(peer) = event {
            assert_eq!(&peer.id, peer_id);
        } else {
            panic!("Event is not a NetworkEvent::PeerJoined: {:?}", event);
        }
    }

    #[derive(Clone, Debug)]
    struct TestNetwork {
        next_address: u64,
        addresses: Vec<Multiaddr>,
    }

    impl TestNetwork {
        pub fn new() -> Self {
            Self {
                next_address: thread_rng().gen::<u64>(),
                addresses: vec![],
            }
        }

        pub async fn spawn(&mut self) -> Network {
            let address = multiaddr![Memory(self.next_address)];
            self.next_address += 1;

            let clock = Arc::new(OffsetTime::new());
            let net = Network::new(
                vec![address.clone()],
                clock,
                network_config(address.clone()),
            )
            .await;
            tracing::debug!(address = ?address, peer_id = ?net.local_peer_id, "creating node");

            if let Some(dial_address) = self.addresses.first() {
                tracing::debug!(address = ?dial_address, "dialing peer");
                net.dial_address(dial_address.clone()).await.unwrap();

                let mut events = net.subscribe_events();
                tracing::debug!("waiting for join event");
                let event = events.next().await;
                tracing::trace!(event = ?event);
            }

            self.addresses.push(address);

            net
        }

        pub async fn spawn_2() -> (Network, Network) {
            let mut net = Self::new();

            let net1 = net.spawn().await;
            let net2 = net.spawn().await;

            (net1, net2)
        }
    }

    async fn create_connected_networks() -> (Network, Network) {
        tracing::debug!("creating connected test networks:");
        let addr1 = multiaddr![Memory(thread_rng().gen::<u64>())];
        let addr2 = multiaddr![Memory(thread_rng().gen::<u64>())];

        let net1 = Network::new(
            vec![addr1.clone()],
            Arc::new(OffsetTime::new()),
            network_config(addr1.clone()),
        )
        .await;
        let net2 = Network::new(
            vec![addr2.clone()],
            Arc::new(OffsetTime::new()),
            network_config(addr2.clone()),
        )
        .await;

        tracing::debug!(address = ?addr1, peer_id = ?net1.local_peer_id, "Network 1");
        tracing::debug!(address = ?addr2, peer_id = ?net2.local_peer_id, "Network 2");

        tracing::debug!("dialing peer 1 from peer 2...");
        net2.dial_address(addr1).await.unwrap();

        let mut events1 = net1.subscribe_events();
        let mut events2 = net2.subscribe_events();

        tracing::debug!("waiting for join events");

        let event1 = events1.next().await.unwrap().unwrap();
        tracing::trace!(event1 = ?event1);
        assert_peer_joined(&event1, &net2.local_peer_id);

        let event2 = events2.next().await.unwrap().unwrap();
        tracing::trace!(event2 = ?event2);
        assert_peer_joined(&event2, &net1.local_peer_id);

        (net1, net2)
    }

    #[tokio::test]
    async fn two_networks_can_connect() {
        let (net1, net2) = create_connected_networks().await;
        assert_eq!(net1.get_peers().len(), 1);
        assert_eq!(net2.get_peers().len(), 1);

        let peer2 = net1.get_peer(*net2.local_peer_id()).unwrap();
        let peer1 = net2.get_peer(*net1.local_peer_id()).unwrap();
        assert_eq!(peer2.id(), net2.local_peer_id);
        assert_eq!(peer1.id(), net1.local_peer_id);
    }

    #[tokio::test]
    async fn one_peer_can_talk_to_another() {
        let (net1, net2) = create_connected_networks().await;

        let peer2 = net1.get_peer(*net2.local_peer_id()).unwrap();
        let peer1 = net2.get_peer(*net1.local_peer_id()).unwrap();

        let mut msgs = peer1.receive::<TestMessage>();

        peer2.send(&TestMessage { id: 4711 }).await.unwrap();

        tracing::debug!("send complete");

        let msg = msgs.next().await.unwrap();

        assert_eq!(msg.id, 4711);
    }

    #[tokio::test]
    async fn one_peer_can_send_multiple_messages() {
        //tracing_subscriber::fmt::init();

        let (net1, net2) = create_connected_networks().await;

        let peer2 = net1.get_peer(*net2.local_peer_id()).unwrap();
        let peer1 = net2.get_peer(*net1.local_peer_id()).unwrap();

        let mut msgs1 = peer1.receive::<TestMessage>();
        let mut msgs2 = peer1.receive::<TestMessage2>();

        peer2.send(&TestMessage { id: 4711 }).await.unwrap();
        peer2
            .send(&TestMessage2 {
                x: "foobar".to_string(),
            })
            .await
            .unwrap();

        tracing::debug!("send complete");

        let msg = msgs1.next().await.unwrap();
        assert_eq!(msg.id, 4711);

        let msg = msgs2.next().await.unwrap();
        assert_eq!(msg.x, "foobar");
    }

    #[tokio::test]
    async fn both_peers_can_talk_with_each_other() {
        let (net1, net2) = create_connected_networks().await;

        let peer2 = net1.get_peer(*net2.local_peer_id()).unwrap();
        let peer1 = net2.get_peer(*net1.local_peer_id()).unwrap();

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
        } else {
            panic!("Event is not a NetworkEvent::PeerLeft: {:?}", event);
        }
    }

    #[ignore]
    #[tokio::test]
    async fn connections_are_properly_closed() {
        //tracing_subscriber::fmt::init();

        let (net1, net2) = create_connected_networks().await;

        //let peer1 = net2.get_peer(net1.local_peer_id().clone()).unwrap();
        let peer2 = net1.get_peer(*net2.local_peer_id()).unwrap();

        let mut events1 = net1.subscribe_events();
        let mut events2 = net2.subscribe_events();

        //peer1.close(CloseReason::Other);
        peer2.close(CloseReason::Other);
        tracing::debug!("closed peer");

        let event1 = events1.next().await.unwrap().unwrap();
        assert_peer_left(&event1, net2.local_peer_id());
        tracing::trace!(event1 = ?event1);

        let event2 = events2.next().await.unwrap().unwrap();
        assert_peer_left(&event2, net1.local_peer_id());
        tracing::trace!(event2 = ?event2);

        assert_eq!(net1.get_peers().len(), 0);
        assert_eq!(net2.get_peers().len(), 0);
    }

    #[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
    pub struct TestRecord {
        x: i32,
    }

    #[tokio::test]
    async fn dht_put_and_get() {
        let (net1, net2) = create_connected_networks().await;

        let put_record = TestRecord { x: 420 };

        net1.dht_put(b"foo", &put_record).await.unwrap();

        let fetched_record = net2.dht_get::<_, TestRecord>(b"foo").await.unwrap();

        assert_eq!(fetched_record, Some(put_record));
    }

    pub struct TestTopic;

    impl Topic for TestTopic {
        type Item = TestRecord;

        fn topic(&self) -> String {
            "hello_world".to_owned()
        }

        fn validate(&self) -> bool {
            true
        }
    }

    fn consume_stream<T: std::fmt::Debug>(
        mut stream: impl Stream<Item = T> + Unpin + Send + 'static,
    ) {
        tokio::spawn(async move { while stream.next().await.is_some() {} });
    }

    #[tokio::test]
    async fn test_gossipsub() {
        tracing_subscriber::fmt::init();

        let mut net = TestNetwork::new();

        let net1 = net.spawn().await;
        let net2 = net.spawn().await;

        for _ in 0..5i32 {
            let net_n = net.spawn().await;
            let stream_n = net_n.subscribe(&TestTopic).await.unwrap();
            consume_stream(stream_n);
        }

        let test_message = TestRecord { x: 42 };

        let mut messages = net1.subscribe(&TestTopic).await.unwrap();
        consume_stream(net2.subscribe(&TestTopic).await.unwrap());

        tokio::time::delay_for(Duration::from_secs(10)).await;

        net2.publish(&TestTopic, test_message.clone())
            .await
            .unwrap();

        tracing::info!("Waiting for GossipSub message...");
        let (received_message, message_id) = messages.next().await.unwrap();
        tracing::info!("Received GossipSub message: {:?}", received_message);

        assert_eq!(received_message, test_message);

        assert!(net1.validate_message(message_id).await.unwrap());
    }
}
