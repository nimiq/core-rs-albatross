use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use bytes::{Buf, Bytes};
use futures::executor;
use futures::{
    future::BoxFuture,
    stream::{BoxStream, StreamExt},
    FutureExt,
};
use libp2p::core::transport::MemoryTransport;
use libp2p::{
    core,
    core::{muxing::StreamMuxerBox, transport::Boxed},
    dns,
    gossipsub::{
        error::PublishError, GossipsubEvent, GossipsubMessage, IdentTopic, MessageAcceptance,
        MessageId, TopicHash, TopicScoreParams,
    },
    identify::IdentifyEvent,
    identity::Keypair,
    kad::{
        store::RecordStore, GetRecordOk, InboundRequest, KademliaEvent, QueryId, QueryResult,
        Quorum, Record,
    },
    noise,
    request_response::{OutboundFailure, RequestId, RequestResponseMessage, ResponseChannel},
    swarm::{dial_opts::DialOpts, ConnectionLimits, NetworkInfo, SwarmBuilder, SwarmEvent},
    tcp, websocket, yamux, Multiaddr, PeerId, Swarm, Transport,
};
use log::Instrument;
use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};

use beserial::{Deserialize, Serialize};
use nimiq_bls::CompressedPublicKey;
use nimiq_network_interface::{
    message::{
        Message, MessageType, RequestError, ResponseError, ResponseMessage, DEFAULT_RESPONSE,
        DEFAULT_RESPONSE_TYPE_ID,
    },
    network::{
        MsgAcceptance, Network as NetworkInterface, NetworkEvent, PubsubId, SubscribeEvents, Topic,
    },
    peer::CloseReason,
};
use nimiq_utils::time::OffsetTime;
use nimiq_validator_network::validator_record::SignedValidatorRecord;

use crate::{
    behaviour::{NimiqBehaviour, NimiqEvent, NimiqNetworkBehaviourError, RequestResponseEvent},
    connection_pool::behaviour::ConnectionPoolEvent,
    dispatch::codecs::typed::{IncomingRequest, OutgoingResponse},
    Config, NetworkError,
};

/// Maximum simultaneous libp2p connections per peer
const MAX_CONNECTIONS_PER_PEER: u32 = 2;

type NimiqSwarm = Swarm<NimiqBehaviour>;
#[derive(Debug)]
pub(crate) enum NetworkAction {
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
        topic_name: &'static str,
        buffer_size: usize,
        validate: bool,
        output: oneshot::Sender<
            Result<mpsc::Receiver<(GossipsubMessage, MessageId, PeerId)>, NetworkError>,
        >,
    },
    Unsubscribe {
        topic_name: &'static str,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    Publish {
        topic_name: &'static str,
        data: Vec<u8>,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    NetworkInfo {
        output: oneshot::Sender<NetworkInfo>,
    },
    ReceiveRequests {
        type_id: MessageType,
        output: mpsc::Sender<(Bytes, RequestId, PeerId)>,
    },
    ListenOn {
        listen_addresses: Vec<Multiaddr>,
    },
    StartConnecting,
    SendRequest {
        peer_id: PeerId,
        request: IncomingRequest,
        response_channel: oneshot::Sender<(ResponseMessage<Bytes>, RequestId, PeerId, MessageType)>,
        output: oneshot::Sender<RequestId>,
    },
    SendResponse {
        request_id: RequestId,
        response: OutgoingResponse,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
}

struct ValidateMessage<P: Clone> {
    pubsub_id: GossipsubId<P>,
    acceptance: MessageAcceptance,
    topic: &'static str,
}

impl<P: Clone> ValidateMessage<P> {
    pub fn new<T>(pubsub_id: GossipsubId<P>, acceptance: MsgAcceptance) -> Self
    where
        T: Topic + Sync,
    {
        Self {
            pubsub_id,
            acceptance: match acceptance {
                MsgAcceptance::Accept => MessageAcceptance::Accept,
                MsgAcceptance::Ignore => MessageAcceptance::Ignore,
                MsgAcceptance::Reject => MessageAcceptance::Reject,
            },
            topic: <T as Topic>::NAME,
        }
    }
}

#[derive(Default)]
struct TaskState {
    dht_puts: HashMap<QueryId, oneshot::Sender<Result<(), NetworkError>>>,
    dht_gets: HashMap<QueryId, oneshot::Sender<Result<Option<Vec<u8>>, NetworkError>>>,
    gossip_topics: HashMap<TopicHash, (mpsc::Sender<(GossipsubMessage, MessageId, PeerId)>, bool)>,
    is_bootstrapped: bool,
    requests: HashMap<
        RequestId,
        oneshot::Sender<(ResponseMessage<Bytes>, RequestId, PeerId, MessageType)>,
    >,
    response_channels: HashMap<RequestId, ResponseChannel<OutgoingResponse>>,
    receive_requests: HashMap<MessageType, mpsc::Sender<(Bytes, RequestId, PeerId)>>,
}

#[derive(Clone, Debug)]
pub struct GossipsubId<P: Clone> {
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
    connected_peers: Arc<RwLock<HashMap<PeerId, Option<oneshot::Sender<CloseReason>>>>>,
    events_tx: broadcast::Sender<NetworkEvent<PeerId>>,
    action_tx: mpsc::Sender<NetworkAction>,
    validate_tx: mpsc::UnboundedSender<ValidateMessage<PeerId>>,
}

impl Network {
    /// Create a new libp2p network instance.
    ///
    /// # Arguments
    ///
    ///  - `clock`: The clock that is used to establish the network time. The discovery behavior will determine the
    ///             offset by exchanging their wall-time with other peers.
    ///  - `config`: The network configuration, containing key pair, and other behavior-specific configuration.
    ///
    pub async fn new(clock: Arc<OffsetTime>, config: Config) -> Self {
        let swarm = Self::new_swarm(clock, config);

        let local_peer_id = *Swarm::local_peer_id(&swarm);
        let connected_peers = Arc::new(RwLock::new(HashMap::new()));

        let (events_tx, _) = broadcast::channel(64);
        let (action_tx, action_rx) = mpsc::channel(64);
        let (validate_tx, validate_rx) = mpsc::unbounded_channel();

        tokio::spawn(Self::swarm_task(
            swarm,
            events_tx.clone(),
            action_rx,
            validate_rx,
            connected_peers.clone(),
        ));

        Self {
            local_peer_id,
            connected_peers,
            events_tx,
            action_tx,
            validate_tx,
        }
    }

    fn new_transport(
        keypair: &Keypair,
        memory_transport: bool,
    ) -> std::io::Result<Boxed<(PeerId, StreamMuxerBox)>> {
        if memory_transport {
            // Memory transport primary for testing
            // TODO: Use websocket over the memory transport
            let transport = websocket::WsConfig::new(dns::TokioDnsConfig::system(
                tcp::TokioTcpConfig::new().nodelay(true),
            )?)
            .or_transport(MemoryTransport::default());

            let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
                .into_authentic(keypair)
                .unwrap();

            let mut yamux = yamux::YamuxConfig::default();
            yamux.set_window_update_mode(yamux::WindowUpdateMode::on_read());

            Ok(transport
                .upgrade(core::upgrade::Version::V1)
                .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
                .multiplex(yamux)
                .timeout(std::time::Duration::from_secs(20))
                .boxed())
        } else {
            let transport = websocket::WsConfig::new(dns::TokioDnsConfig::system(
                tcp::TokioTcpConfig::new().nodelay(true),
            )?);

            let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
                .into_authentic(keypair)
                .unwrap();

            let mut yamux = yamux::YamuxConfig::default();
            yamux.set_window_update_mode(yamux::WindowUpdateMode::on_read());

            Ok(transport
                .upgrade(core::upgrade::Version::V1)
                .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
                .multiplex(yamux)
                .timeout(std::time::Duration::from_secs(20))
                .boxed())
        }
    }

    fn new_swarm(clock: Arc<OffsetTime>, config: Config) -> Swarm<NimiqBehaviour> {
        let local_peer_id = PeerId::from(config.keypair.public());

        let transport = Self::new_transport(&config.keypair, config.memory_transport).unwrap();

        let behaviour = NimiqBehaviour::new(config, clock);

        let limits = ConnectionLimits::default()
            .with_max_pending_incoming(Some(16))
            .with_max_pending_outgoing(Some(16))
            .with_max_established_incoming(Some(4800))
            .with_max_established_outgoing(Some(4800))
            .with_max_established_per_peer(Some(MAX_CONNECTIONS_PER_PEER));

        // TODO add proper config
        SwarmBuilder::new(transport, behaviour, local_peer_id)
            .connection_limits(limits)
            .executor(Box::new(|fut| {
                tokio::spawn(fut);
            }))
            .build()
    }

    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    async fn swarm_task(
        mut swarm: NimiqSwarm,
        events_tx: broadcast::Sender<NetworkEvent<PeerId>>,
        mut action_rx: mpsc::Receiver<NetworkAction>,
        mut validate_rx: mpsc::UnboundedReceiver<ValidateMessage<PeerId>>,
        connected_peers: Arc<RwLock<HashMap<PeerId, Option<oneshot::Sender<CloseReason>>>>>,
    ) {
        let mut task_state = TaskState::default();

        let peer_id = Swarm::local_peer_id(&swarm);
        let task_span = log::trace_span!("swarm task", peer_id=?peer_id);

        async move {
            loop {
                tokio::select! {
                    validate_msg = validate_rx.recv() => {
                        if let Some(validate_msg) = validate_msg {
                            let topic = validate_msg.topic;
                            let result: Result<bool, PublishError> = swarm
                                .behaviour_mut()
                                .gossipsub
                                .report_message_validation_result(
                                    &validate_msg.pubsub_id.message_id,
                                    &validate_msg.pubsub_id.propagation_source,
                                    validate_msg.acceptance,
                                );

                            match result {
                                Ok(true) => {}, // success
                                Ok(false) => log::debug!("Validation took too long: the {} message is no longer in the message cache", topic),
                                Err(e) => log::error!("Network error while relaying {} message: {}", topic, e),
                            }
                        }
                    },
                    event = swarm.next() => {
                        if let Some(event) = event {
                            Self::handle_event(event, &events_tx, &mut swarm, &mut task_state, &connected_peers);
                        }
                    },
                    action = action_rx.recv() => {
                        if let Some(action) = action {
                            Self::perform_action(action, &mut swarm, &mut task_state);
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

    fn handle_event(
        event: SwarmEvent<NimiqEvent, NimiqNetworkBehaviourError>,
        events_tx: &broadcast::Sender<NetworkEvent<PeerId>>,
        swarm: &mut NimiqSwarm,
        state: &mut TaskState,
        connected_peers: &RwLock<HashMap<PeerId, Option<oneshot::Sender<CloseReason>>>>,
    ) {
        match event {
            SwarmEvent::ConnectionEstablished {
                peer_id,
                endpoint,
                num_established,
                concurrent_dial_errors,
            } => {
                log::info!(
                    "Connection established with peer {}, {:?}, connections established: {:?}",
                    peer_id,
                    endpoint,
                    num_established
                );

                if let Some(dial_errors) = concurrent_dial_errors {
                    for (addr, error) in dial_errors {
                        log::debug!(
                            "Failed to reach address: {}, peer_id={:?}, error={:?}",
                            addr,
                            peer_id,
                            error
                        );
                        swarm.behaviour_mut().remove_peer_address(peer_id, addr);
                    }
                }

                // Save dialed peer addresses
                if endpoint.is_dialer() {
                    let listen_addr = endpoint.get_remote_address();

                    log::debug!("Saving peer {} listen address: {:?}", peer_id, listen_addr);

                    swarm
                        .behaviour_mut()
                        .add_peer_address(peer_id, listen_addr.clone());

                    // Bootstrap Kademlia if we're performing our first connection
                    if !state.is_bootstrapped {
                        log::debug!("Bootstrapping DHT");
                        if swarm.behaviour_mut().dht.bootstrap().is_err() {
                            log::error!("Bootstrapping DHT error: No known peers");
                        }
                        state.is_bootstrapped = true;
                    }
                }
            }

            SwarmEvent::ConnectionClosed {
                peer_id,
                endpoint,
                num_established,
                cause,
            } => {
                log::info!(
                    "Connection closed with peer {}, {:?}, connections established: {:?}",
                    peer_id,
                    endpoint,
                    num_established
                );

                if let Some(cause) = cause {
                    log::info!("Connection closed because: {:?}", cause);
                }

                // Remove Peer
                connected_peers.write().remove(&peer_id);
                swarm.behaviour_mut().remove_peer(peer_id);
                events_tx.send(NetworkEvent::PeerLeft(peer_id)).ok();
            }

            SwarmEvent::IncomingConnection {
                local_addr,
                send_back_addr,
            } => {
                log::debug!(
                    "Incoming connection from address {:?} to listen address {:?}",
                    send_back_addr,
                    local_addr
                );
            }

            SwarmEvent::IncomingConnectionError {
                local_addr,
                send_back_addr,
                error,
            } => {
                log::debug!(
                    "Incoming connection error from address {:?} to listen address {:?}: {:?}",
                    send_back_addr,
                    local_addr,
                    error
                );
            }

            SwarmEvent::Dialing(peer_id) => {
                // This event is only triggered if the network behaviour performs the dial
                log::debug!("Dialing peer {}", peer_id);
            }

            SwarmEvent::Behaviour(event) => {
                match event {
                    NimiqEvent::Dht(event) => {
                        match event {
                            KademliaEvent::OutboundQueryCompleted { id, result, .. } => {
                                match result {
                                    QueryResult::GetRecord(result) => {
                                        if let Some(output) = state.dht_gets.remove(&id) {
                                            let result = result.map_err(Into::into).map(
                                                |GetRecordOk { mut records, .. }| {
                                                    // TODO: What do we do, if we get multiple records?
                                                    records.pop().map(|r| r.record.value)
                                                },
                                            );
                                            output.send(result).ok();
                                        } else {
                                            log::warn!(query_id = ?id, "GetRecord query result for unknown query ID");
                                        }
                                    }
                                    QueryResult::PutRecord(result) => {
                                        // dht_put resolved
                                        if let Some(output) = state.dht_puts.remove(&id) {
                                            output
                                                .send(result.map(|_| ()).map_err(Into::into))
                                                .ok();
                                        } else {
                                            log::warn!(query_id = ?id, "PutRecord query result for unknown query ID");
                                        }
                                    }
                                    QueryResult::Bootstrap(result) => match result {
                                        Ok(result) => {
                                            log::debug!(result = ?result, "DHT bootstrap successful")
                                        }
                                        Err(e) => log::error!("DHT bootstrap error: {:?}", e),
                                    },
                                    _ => {}
                                }
                            }
                            KademliaEvent::InboundRequest {
                                request:
                                    InboundRequest::PutRecord {
                                        source: _,
                                        connection: _,
                                        record: Some(record),
                                    },
                            } => {
                                if let Ok(compressed_pk) =
                                    <[u8; 285]>::try_from(record.key.as_ref())
                                {
                                    if let Ok(pk) = (CompressedPublicKey {
                                        public_key: compressed_pk,
                                    })
                                    .uncompress()
                                    {
                                        if let Ok(signed_record) =
                                            SignedValidatorRecord::<PeerId>::deserialize_from_vec(
                                                &record.value,
                                            )
                                        {
                                            if signed_record.verify(&pk) {
                                                if swarm
                                                    .behaviour_mut()
                                                    .dht
                                                    .store_mut()
                                                    .put(record)
                                                    .is_ok()
                                                {
                                                    return;
                                                } else {
                                                    log::error!("Could not store record in DHT record store");
                                                    return;
                                                };
                                            } else {
                                                log::warn!("DHT record signature verification failed. Record public key: {:?}", pk);
                                                return;
                                            }
                                        }
                                    }
                                }
                                log::warn!(
                                    "DHT record verification failed: Invalid public key received"
                                );
                            }
                            _ => {}
                        }
                    }
                    NimiqEvent::Discovery(_e) => {}
                    NimiqEvent::Gossip(event) => match event {
                        GossipsubEvent::Message {
                            propagation_source,
                            message_id,
                            message,
                        } => {
                            if let Some(topic_info) = state.gossip_topics.get_mut(&message.topic) {
                                let (output, validate) = topic_info;
                                if !&*validate {
                                    swarm
                                        .behaviour_mut()
                                        .gossipsub
                                        .report_message_validation_result(
                                            &message_id,
                                            &propagation_source,
                                            MessageAcceptance::Accept,
                                        )
                                        .ok();
                                }

                                let topic = message.topic.clone();
                                if let Err(e) =
                                    output.try_send((message, message_id, propagation_source))
                                {
                                    log::error!(
                                        "Failed to dispatch gossipsub '{}' message: {:?}",
                                        topic.as_str(),
                                        e
                                    )
                                }
                            } else {
                                log::warn!(topic = ?message.topic, "unknown topic hash");
                            }
                        }
                        GossipsubEvent::Subscribed { peer_id, topic } => {
                            log::debug!(peer_id = ?peer_id, topic = ?topic, "peer subscribed to topic");
                        }
                        GossipsubEvent::Unsubscribed { peer_id, topic } => {
                            log::debug!(peer_id = ?peer_id, topic = ?topic, "peer unsubscribed");
                        }
                        GossipsubEvent::GossipsubNotSupported { peer_id } => {
                            log::debug!(peer_id = ?peer_id, "gossipsub not supported");
                        }
                    },
                    NimiqEvent::Identify(event) => {
                        match event {
                            IdentifyEvent::Received { peer_id, info } => {
                                log::debug!(
                                    "Received identity from peer {} at address {:?}: {:?}",
                                    peer_id,
                                    info.observed_addr,
                                    info
                                );

                                // Save identified peer listen addresses
                                for listen_addr in info.listen_addrs {
                                    swarm.behaviour_mut().add_peer_address(peer_id, listen_addr);

                                    // Bootstrap Kademlia if we're adding our first address
                                    if !state.is_bootstrapped {
                                        log::debug!("Bootstrapping DHT");
                                        if swarm.behaviour_mut().dht.bootstrap().is_err() {
                                            log::error!("Bootstrapping DHT error: No known peers");
                                        }
                                        state.is_bootstrapped = true;
                                    }
                                }
                            }
                            IdentifyEvent::Pushed { peer_id } => {
                                log::trace!("Pushed identity to peer {}", peer_id);
                            }
                            IdentifyEvent::Sent { peer_id } => {
                                log::trace!("Sent identity to peer {}", peer_id);
                            }
                            IdentifyEvent::Error { peer_id, error } => {
                                log::error!(
                                    "Error while identifying remote peer {}: {:?}",
                                    peer_id,
                                    error
                                );
                            }
                        }
                    }
                    NimiqEvent::Pool(event) => {
                        match event {
                            ConnectionPoolEvent::PeerJoined { peer_id, close_tx } => {
                                if connected_peers
                                    .write()
                                    .insert(peer_id, Some(close_tx))
                                    .is_some()
                                {
                                    log::error!(?peer_id, "peer joined but it already exists");
                                }
                                events_tx.send(NetworkEvent::PeerJoined(peer_id)).ok();
                            }
                        };
                    }
                    NimiqEvent::RequestResponse(event) => match event {
                        RequestResponseEvent::Message {
                            peer: peer_id,
                            message,
                        } => match message {
                            RequestResponseMessage::Request {
                                request_id,
                                request: request_data,
                                channel,
                            } => {
                                // TODO Add rate limiting (per peer).
                                let (type_id, request) = request_data;
                                log::trace!(
                                    "Incoming [Request ID {}] from peer {}: {:?}",
                                    request_id,
                                    peer_id,
                                    request,
                                );
                                // Check if we have a receiver registered for this message type
                                let sender = {
                                    if let Some(sender) = state.receive_requests.get_mut(&type_id) {
                                        // Check if the sender is still alive, if not remove it
                                        if sender.is_closed() {
                                            state.receive_requests.remove(&type_id);
                                            None
                                        } else {
                                            Some(sender)
                                        }
                                    } else {
                                        None
                                    }
                                };
                                // If we have a receiver, pass the request. Otherwise send the default response
                                if let Some(sender) = sender {
                                    state.response_channels.insert(request_id, channel);
                                    if let Err(e) =
                                        sender.try_send((request.freeze(), request_id, peer_id))
                                    {
                                        log::error!(
                                            "Failed to dispatch [Request ID {}] from peer {}: {:?}",
                                            request_id,
                                            peer_id,
                                            e,
                                        );
                                    }
                                } else {
                                    log::trace!(
                                        "No receiver found for requests of type ID {}, replying with default response",
                                        type_id
                                    );
                                    if let Err((message_type, _)) =
                                        swarm.behaviour_mut().request_response.send_response(
                                            channel,
                                            (DEFAULT_RESPONSE_TYPE_ID, DEFAULT_RESPONSE[..].into()),
                                        )
                                    {
                                        log::error!(
                                            "Could not send default response for message type {}",
                                            message_type
                                        );
                                    };
                                }
                            }
                            RequestResponseMessage::Response {
                                request_id,
                                response,
                            } => {
                                log::trace!(
                                    "[Request ID {}] Incoming response from peer {}",
                                    request_id,
                                    peer_id,
                                );
                                let (message_type, response) = response;
                                if let Some(channel) = state.requests.remove(&request_id) {
                                    channel
                                        .send((
                                            ResponseMessage::Response(response.freeze()),
                                            request_id,
                                            peer_id,
                                            message_type,
                                        ))
                                        .ok();
                                } else {
                                    log::error!(
                                        "No such request ID found: [Request ID {}]",
                                        request_id
                                    );
                                }
                            }
                        },
                        RequestResponseEvent::OutboundFailure {
                            peer,
                            request_id,
                            error,
                        } => {
                            log::error!(
                                "[Request ID {}] sent to peer {} failed, error: {:?}",
                                request_id,
                                peer,
                                error,
                            );
                            if let Some(channel) = state.requests.remove(&request_id) {
                                channel
                                    .send((
                                        ResponseMessage::<Bytes>::Error(Self::to_response_error(
                                            error,
                                        )),
                                        request_id,
                                        peer,
                                        0.into(),
                                    ))
                                    .ok();
                            } else {
                                log::error!(
                                    "No such request ID found: [Request ID {}]",
                                    request_id
                                );
                            }
                        }
                        RequestResponseEvent::InboundFailure {
                            peer,
                            request_id,
                            error,
                        } => {
                            log::error!(
                                "Response to [Request ID {}] from peer {} failed, error: {:?}",
                                request_id,
                                peer,
                                error,
                            );
                        }
                        RequestResponseEvent::ResponseSent { peer, request_id } => {
                            log::trace!(
                                "Response to [Request ID {}] sent to peer: {}",
                                request_id,
                                peer
                            );
                        }
                    },
                }
            }
            _ => {}
        }
    }

    fn perform_action(action: NetworkAction, swarm: &mut NimiqSwarm, state: &mut TaskState) {
        // FIXME implement compact debug format for NetworkAction
        // log::trace!(action = ?action, "performing action");

        match action {
            NetworkAction::Dial { peer_id, output } => {
                output
                    .send(
                        Swarm::dial(swarm, DialOpts::peer_id(peer_id).build()).map_err(Into::into),
                    )
                    .ok();
            }
            NetworkAction::DialAddress { address, output } => {
                output
                    .send(
                        Swarm::dial(swarm, DialOpts::unknown_peer_id().address(address).build())
                            .map_err(Into::into),
                    )
                    .ok();
            }
            NetworkAction::DhtGet { key, output } => {
                let query_id = swarm
                    .behaviour_mut()
                    .dht
                    .get_record(key.into(), Quorum::One);
                state.dht_gets.insert(query_id, output);
            }
            NetworkAction::DhtPut { key, value, output } => {
                let local_peer_id = Swarm::local_peer_id(swarm);

                let record = Record {
                    key: key.into(),
                    value,
                    publisher: Some(*local_peer_id),
                    expires: None, // TODO: Records should expire at some point in time
                };

                match swarm.behaviour_mut().dht.put_record(record, Quorum::One) {
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
                buffer_size,
                validate,
                output,
            } => {
                let topic = IdentTopic::new(topic_name);

                match swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                    // New subscription. Insert the sender into our subscription table.
                    Ok(true) => {
                        let (tx, rx) = mpsc::channel(buffer_size);

                        state.gossip_topics.insert(topic.hash(), (tx, validate));

                        match swarm
                            .behaviour_mut()
                            .gossipsub
                            .set_topic_params(topic, TopicScoreParams::default())
                        {
                            Ok(_) => output.send(Ok(rx)).ok(),
                            Err(e) => output
                                .send(Err(NetworkError::TopicScoreParams {
                                    topic_name,
                                    error: e,
                                }))
                                .ok(),
                        };
                    }

                    // Apparently we're already subscribed.
                    Ok(false) => {
                        output
                            .send(Err(NetworkError::AlreadySubscribed { topic_name }))
                            .ok();
                    }

                    // Subscribe failed. Send back error.
                    Err(e) => {
                        output.send(Err(e.into())).ok();
                    }
                }
            }
            NetworkAction::Unsubscribe { topic_name, output } => {
                let topic = IdentTopic::new(topic_name);

                if state.gossip_topics.get_mut(&topic.hash()).is_some() {
                    match swarm.behaviour_mut().gossipsub.unsubscribe(&topic) {
                        // Unsubscription. Remove the topic from the subscription table.
                        Ok(true) => {
                            drop(state.gossip_topics.remove(&topic.hash()).unwrap().0);

                            output.send(Ok(())).ok();
                        }

                        // Apparently we're already unsubscribed.
                        Ok(false) => {
                            drop(state.gossip_topics.remove(&topic.hash()).unwrap().0);

                            output
                                .send(Err(NetworkError::AlreadyUnsubscribed { topic_name }))
                                .ok();
                        }

                        // Unsubscribe failed. Send back error.
                        Err(e) => {
                            output.send(Err(e.into())).ok();
                        }
                    }
                } else {
                    // If the topic wasn't in the topics list, we're not subscribed to it.
                    output
                        .send(Err(NetworkError::AlreadyUnsubscribed { topic_name }))
                        .ok();
                }
            }
            NetworkAction::Publish {
                topic_name,
                data,
                output,
            } => {
                let topic = IdentTopic::new(topic_name);

                output
                    .send(
                        swarm
                            .behaviour_mut()
                            .gossipsub
                            .publish(topic, data)
                            .map(|_| ())
                            .or_else(|e| match e {
                                PublishError::Duplicate => Ok(()),
                                _ => Err(e),
                            })
                            .map_err(Into::into),
                    )
                    .ok();
            }
            NetworkAction::NetworkInfo { output } => {
                output.send(Swarm::network_info(swarm)).ok();
            }
            NetworkAction::ReceiveRequests { type_id, output } => {
                state.receive_requests.insert(type_id, output);
            }
            NetworkAction::ListenOn { listen_addresses } => {
                for listen_address in listen_addresses {
                    Swarm::listen_on(swarm, listen_address)
                        .expect("Failed to listen on provided address");
                }
            }
            NetworkAction::StartConnecting => {
                swarm.behaviour_mut().pool.start_connecting();
            }
            NetworkAction::SendRequest {
                peer_id,
                request,
                response_channel,
                output,
            } => {
                let type_id = request.0;
                let request_id = swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer_id, request);
                log::trace!(
                    "[Request ID {}] was sent to peer {}, type id {}",
                    request_id,
                    peer_id,
                    type_id
                );
                state.requests.insert(request_id, response_channel);
                output.send(request_id).ok();
            }
            NetworkAction::SendResponse {
                request_id,
                response,
                output,
            } => {
                if let Some(response_channel) = state.response_channels.remove(&request_id) {
                    if let Err(e) = output.send(
                        swarm
                            .behaviour_mut()
                            .request_response
                            .send_response(response_channel, response)
                            .map_err(NetworkError::ResponseChannelClosed),
                    ) {
                        log::error!(
                            "Response was sent but the action channel was dropped: {:?}",
                            e
                        );
                    };
                } else {
                    log::error!("Tried to respond to a non existing request");
                    output.send(Err(NetworkError::UnknownRequestId)).ok();
                }
            }
        }
    }

    pub async fn network_info(&self) -> Result<NetworkInfo, NetworkError> {
        let (output_tx, output_rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::NetworkInfo { output: output_tx })
            .await?;
        Ok(output_rx.await?)
    }

    pub async fn listen_on(&self, listen_addresses: Vec<Multiaddr>) {
        self.action_tx
            .clone()
            .send(NetworkAction::ListenOn { listen_addresses })
            .await
            .map_err(|e| log::error!("Failed to send NetworkAction::ListenOnAddress: {:?}", e))
            .ok();
    }

    pub async fn start_connecting(&self) {
        self.action_tx
            .clone()
            .send(NetworkAction::StartConnecting)
            .await
            .map_err(|e| log::error!("Failed to send NetworkAction::StartConnecting: {:?}", e))
            .ok();
    }

    fn to_response_error(error: OutboundFailure) -> ResponseError {
        match error {
            OutboundFailure::ConnectionClosed => ResponseError::ConnectionClosed,
            OutboundFailure::DialFailure => ResponseError::DialFailure,
            OutboundFailure::Timeout => ResponseError::Timeout,
            OutboundFailure::UnsupportedProtocols => ResponseError::UnsupportedProtocols,
        }
    }
    pub fn disconnect(&self) {
        for peer_id in self.get_peers() {
            self.disconnect_peer(peer_id, CloseReason::Other);
        }
    }
}

#[async_trait]
impl NetworkInterface for Network {
    type PeerId = PeerId;
    type AddressType = Multiaddr;
    type Error = NetworkError;
    type PubsubId = GossipsubId<PeerId>;
    type RequestId = RequestId;

    fn get_peers(&self) -> Vec<PeerId> {
        self.connected_peers.read().keys().copied().collect()
    }

    fn has_peer(&self, peer_id: PeerId) -> bool {
        self.connected_peers.read().contains_key(&peer_id)
    }

    fn disconnect_peer(&self, peer_id: PeerId, close_reason: CloseReason) {
        if let Some(maybe_close_tx) = self.connected_peers.write().get_mut(&peer_id) {
            if let Some(close_tx) = maybe_close_tx.take() {
                if let Err(_) = close_tx.send(close_reason) {
                    log::error!("The receiver for disconnect_peer was already dropped.");
                }
            } else {
                log::error!("Peer is already closed");
            }
        }
    }

    fn subscribe_events(&self) -> SubscribeEvents<PeerId> {
        Box::pin(BroadcastStream::new(self.events_tx.subscribe()))
    }

    async fn subscribe<T>(
        &self,
    ) -> Result<BoxStream<'static, (T::Item, Self::PubsubId)>, Self::Error>
    where
        T: Topic + Sync,
    {
        let (tx, rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::Subscribe {
                topic_name: <T as Topic>::NAME,
                buffer_size: <T as Topic>::BUFFER_SIZE,
                validate: <T as Topic>::VALIDATE,
                output: tx,
            })
            .await?;

        // Receive the mpsc::Receiver, but propagate errors first.
        let subscribe_rx = ReceiverStream::new(rx.await??);

        Ok(Box::pin(subscribe_rx.map(|(msg, msg_id, source)| {
            let item: <T as Topic>::Item = Deserialize::deserialize_from_vec(&msg.data).unwrap();
            let id = GossipsubId {
                message_id: msg_id,
                propagation_source: source,
            };
            (item, id)
        })))
    }

    async fn unsubscribe<T>(&self) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let (output_tx, output_rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::Unsubscribe {
                topic_name: <T as Topic>::NAME,
                output: output_tx,
            })
            .await?;

        output_rx.await?
    }

    async fn publish<T>(&self, item: <T as Topic>::Item) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let (output_tx, output_rx) = oneshot::channel();

        let mut buf = vec![];
        item.serialize(&mut buf)?;

        self.action_tx
            .clone()
            .send(NetworkAction::Publish {
                topic_name: <T as Topic>::NAME,
                data: buf,
                output: output_tx,
            })
            .await?;

        output_rx.await??;

        Ok(())
    }

    fn validate_message<T>(&self, pubsub_id: Self::PubsubId, acceptance: MsgAcceptance)
    where
        T: Topic + Sync,
    {
        self.validate_tx
            .send(ValidateMessage::new::<T>(pubsub_id, acceptance))
            .ok()
            .expect("Failed to send reported message validation result");
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

    fn get_local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    async fn request<Req: Message, Res: Message>(
        &self,
        request: Req,
        peer_id: PeerId,
    ) -> Result<BoxFuture<'static, (ResponseMessage<Res>, RequestId, PeerId)>, RequestError> {
        let (output_tx, output_rx) = oneshot::channel();
        let (response_tx, response_rx) = oneshot::channel();

        let mut buf = vec![];
        request
            .serialize(&mut buf)
            .map_err(|_| RequestError::SerializationError)?;

        self.action_tx
            .clone()
            .send(NetworkAction::SendRequest {
                peer_id,
                request: ((Req::TYPE_ID as u64).into(), buf[..].into()),
                response_channel: response_tx,
                output: output_tx,
            })
            .await
            .map_err(|_| RequestError::SendError)?;

        let request_id = output_rx.await.map_err(|_| RequestError::SendError)?;

        let mapped_future = response_rx.map(move |data| {
            async move {
                match data {
                    Err(_) => (ResponseMessage::Error(ResponseError::SenderFutureDropped), request_id, peer_id),
                    Ok((data, request_id, peer_id, message_type)) => {
                        match data {
                            ResponseMessage::Response(data) => {
                                if message_type == (Res::TYPE_ID as u64).into() {
                                    match <Res as Deserialize>::deserialize(&mut data.reader()) {
                                        Ok(message) => {
                                            (ResponseMessage::Response(message), request_id, peer_id)
                                        },
                                        Err(e) => {
                                            log::error!(
                                                "Failed to deserialize request ID {} of type {} message from {}: {}",
                                                request_id,
                                                std::any::type_name::<Res>(),
                                                peer_id,
                                                e
                                                );
                                            (ResponseMessage::Error(ResponseError::DeSerializationError), request_id, peer_id)
                                        },
                                    }
                                } else if message_type == DEFAULT_RESPONSE_TYPE_ID {
                                    (ResponseMessage::Error(ResponseError::NoReceiver), request_id, peer_id)
                                } else {
                                    (ResponseMessage::Error(ResponseError::InvalidResponse), request_id, peer_id)
                                }
                            },
                            ResponseMessage::Error(e) => (ResponseMessage::Error(e), request_id, peer_id),
                        }
                    }
                }
            }
        }).flatten();

        Ok(mapped_future.boxed())
    }

    fn receive_requests<M: Message>(&self) -> BoxStream<'static, (M, RequestId, PeerId)> {
        let action_tx = self.action_tx.clone();

        // Future to register the channel.
        let register_future = async move {
            // TODO Make buffer size configurable
            let (tx, rx) = mpsc::channel(1024);

            action_tx
                .send(NetworkAction::ReceiveRequests {
                    type_id: (M::TYPE_ID as u64).into(),
                    output: tx,
                })
                .await
                .expect("Sending action to network task failed.");

            rx
        };

        // XXX Drive the register future to completion. This is needed because we want the receivers
        // to be properly set up when this function returns. It should be ok to block here as we're
        // only calling this during client initialization.
        // A better way to do this would be make receive_from_all() async.
        let receive_stream = ReceiverStream::new(executor::block_on(register_future));

        receive_stream
            .filter_map(|(data, request_id, peer_id)| async move {
                // Map the (data, peer) stream to (message, peer) by deserializing the messages.
                match <M as Deserialize>::deserialize(&mut data.reader()) {
                    Ok(message) => Some((message, request_id, peer_id)),
                    Err(e) => {
                        log::error!(
                            "Failed to deserialize request ID {} of type {} message from {}: {}",
                            request_id,
                            std::any::type_name::<M>(),
                            peer_id,
                            e
                        );
                        None
                    }
                }
            })
            .boxed()
    }

    async fn respond<M: Message>(
        &self,
        request_id: RequestId,
        response: M,
    ) -> Result<(), Self::Error> {
        let (output_tx, output_rx) = oneshot::channel();

        let mut buf = vec![];
        response.serialize(&mut buf)?;

        self.action_tx
            .clone()
            .send(NetworkAction::SendResponse {
                request_id,
                response: ((M::TYPE_ID as u64).into(), buf[..].into()),
                output: output_tx,
            })
            .await?;

        output_rx.await?
    }
}
