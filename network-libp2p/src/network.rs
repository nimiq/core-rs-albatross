use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_trait::async_trait;
use base64::Engine;
use bytes::{Buf, Bytes};
use futures::{ready, stream::BoxStream, Stream, StreamExt};
#[cfg(not(feature = "tokio-time"))]
use instant::Instant;
use libp2p::core::transport::MemoryTransport;
use libp2p::gossipsub::PeerScoreParams;
use libp2p::{
    core,
    core::{muxing::StreamMuxerBox, transport::Boxed},
    gossipsub::{
        error::PublishError, GossipsubEvent, GossipsubMessage, IdentTopic, MessageAcceptance,
        MessageId, TopicHash, TopicScoreParams,
    },
    identify::Event as IdentifyEvent,
    identity::Keypair,
    kad::{
        store::RecordStore, GetRecordOk, InboundRequest, KademliaEvent, QueryId, QueryResult,
        Quorum, Record,
    },
    noise,
    ping::Success as PingSuccess,
    request_response::{OutboundFailure, RequestId, RequestResponseMessage, ResponseChannel},
    swarm::{dial_opts::DialOpts, ConnectionLimits, NetworkInfo, SwarmBuilder, SwarmEvent},
    yamux, Multiaddr, PeerId, Swarm, Transport,
};
#[cfg(feature = "tokio-websocket")]
use libp2p::{dns, tcp, websocket};
#[cfg(all(feature = "wasm-websocket", not(feature = "tokio-websocket")))]
use libp2p_websys_transport::WebsocketTransport;
use log::Instrument;
use parking_lot::{Mutex, RwLock};
use tokio::sync::{broadcast, mpsc, oneshot};
#[cfg(feature = "tokio-time")]
use tokio::time::{Instant, Interval};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
#[cfg(not(feature = "tokio-time"))]
use wasm_timer::Interval;

use beserial::{Deserialize, Serialize};
use nimiq_bls::CompressedPublicKey;
use nimiq_network_interface::{
    network::{
        CloseReason, MsgAcceptance, Network as NetworkInterface, NetworkEvent, PubsubId,
        SubscribeEvents, Topic,
    },
    peer_info::{PeerInfo, Services},
    request::{
        peek_type, InboundRequestError, Message, OutboundRequestError, Request, RequestCommon,
        RequestError, RequestType,
    },
};
use nimiq_primitives::task_executor::TaskExecutor;
use nimiq_utils::time::OffsetTime;
use nimiq_validator_network::validator_record::SignedValidatorRecord;

use crate::discovery::behaviour::DiscoveryEvent;
use crate::discovery::peer_contacts::PeerContactBook;
#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::rate_limiting::PendingDeletion;
use crate::rate_limiting::RateLimit;
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
        type_id: RequestType,
        output: mpsc::Sender<(Bytes, RequestId, PeerId)>,
    },
    SendRequest {
        peer_id: PeerId,
        request: IncomingRequest,
        request_type_id: RequestType,
        response_channel: oneshot::Sender<Result<Bytes, RequestError>>,
        output: oneshot::Sender<RequestId>,
    },
    SendResponse {
        request_id: RequestId,
        response: OutgoingResponse,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    ListenOn {
        listen_addresses: Vec<Multiaddr>,
    },
    StartConnecting,
    DisconnectPeer {
        peer_id: PeerId,
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
    requests: HashMap<RequestId, oneshot::Sender<Result<Bytes, RequestError>>>,
    #[cfg(feature = "metrics")]
    requests_initiated: HashMap<RequestId, Instant>,
    response_channels: HashMap<RequestId, ResponseChannel<OutgoingResponse>>,
    receive_requests: HashMap<RequestType, mpsc::Sender<(Bytes, RequestId, PeerId)>>,
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
    /// The local ID that is used to identify our peer
    local_peer_id: PeerId,
    /// This hash map maintains an association between PeerIds and PeerInfo:
    /// If the peer is interesting, i.e.: it provides services that are interested to us,
    /// we store an entry with the peer contact itself.
    connected_peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
    /// Stream used to send event messages
    events_tx: broadcast::Sender<NetworkEvent<PeerId>>,
    /// Stream used to send action messages
    action_tx: mpsc::Sender<NetworkAction>,
    /// Stream used to send validation messages
    validate_tx: mpsc::UnboundedSender<ValidateMessage<PeerId>>,
    /// Maintains the rate limits being enforced for our peers. The limits are enforced by
    /// peer_id and request type.
    peer_request_limits: Arc<Mutex<HashMap<PeerId, HashMap<u16, RateLimit>>>>,
    /// Metrics used for data analysis
    #[cfg(feature = "metrics")]
    metrics: Arc<NetworkMetrics>,
    /// Required services from other peers. This is defined on init, based on our client type
    required_services: Services,
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
    pub async fn new(
        clock: Arc<OffsetTime>,
        config: Config,
        executor: impl TaskExecutor + Send + Clone + 'static,
    ) -> Self {
        let required_services = config.required_services;
        // TODO: persist to disk
        let own_peer_contact = config.peer_contact.clone();
        let contacts = Arc::new(RwLock::new(PeerContactBook::new(
            own_peer_contact.sign(&config.keypair),
        )));
        let params = PeerScoreParams {
            ip_colocation_factor_threshold: 20.0,
            ..Default::default()
        };
        let swarm = Self::new_swarm(
            clock,
            config,
            Arc::clone(&contacts),
            params.clone(),
            executor.clone(),
        );

        let local_peer_id = *Swarm::local_peer_id(&swarm);
        let connected_peers = Arc::new(RwLock::new(HashMap::new()));

        let (events_tx, _) = broadcast::channel(64);
        let (action_tx, action_rx) = mpsc::channel(64);
        let (validate_tx, validate_rx) = mpsc::unbounded_channel();
        let peer_request_limits = Arc::new(Mutex::new(HashMap::new()));
        let rate_limits_pending_deletion = Arc::new(Mutex::new(PendingDeletion::default()));

        #[cfg(not(feature = "tokio-time"))]
        let update_scores = wasm_timer::Interval::new(params.decay_interval);
        #[cfg(feature = "tokio-time")]
        let update_scores = tokio::time::interval(params.decay_interval);

        #[cfg(feature = "metrics")]
        let metrics = Arc::new(NetworkMetrics::default());

        executor.exec(Box::pin(Self::swarm_task(
            swarm,
            events_tx.clone(),
            action_rx,
            validate_rx,
            Arc::clone(&connected_peers),
            Arc::clone(&peer_request_limits),
            Arc::clone(&rate_limits_pending_deletion),
            update_scores,
            contacts,
            #[cfg(feature = "metrics")]
            metrics.clone(),
        )));

        Self {
            local_peer_id,
            connected_peers,
            events_tx,
            action_tx,
            validate_tx,
            peer_request_limits,
            #[cfg(feature = "metrics")]
            metrics,
            required_services,
        }
    }

    fn new_transport(
        keypair: &Keypair,
        memory_transport: bool,
    ) -> std::io::Result<Boxed<(PeerId, StreamMuxerBox)>> {
        if memory_transport {
            // Memory transport primary for testing
            // TODO: Use websocket over the memory transport

            #[cfg(feature = "tokio-websocket")]
            let transport = websocket::WsConfig::new(dns::TokioDnsConfig::system(
                tcp::tokio::Transport::new(tcp::Config::default().nodelay(true)),
            )?)
            .or_transport(MemoryTransport::default());
            #[cfg(not(feature = "tokio-websocket"))]
            let transport = MemoryTransport::default();
            // Fixme: Handle wasm compatible transport

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
            #[cfg(feature = "tokio-websocket")]
            let transport = websocket::WsConfig::new(dns::TokioDnsConfig::system(
                tcp::tokio::Transport::new(tcp::Config::default().nodelay(true)),
            )?);
            #[cfg(all(feature = "wasm-websocket", not(feature = "tokio-websocket")))]
            let transport = WebsocketTransport::default();
            #[cfg(all(not(feature = "tokio-websocket"), not(feature = "wasm-websocket")))]
            let transport = MemoryTransport::default();

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

    fn new_swarm(
        clock: Arc<OffsetTime>,
        config: Config,
        contacts: Arc<RwLock<PeerContactBook>>,
        peer_score_params: PeerScoreParams,
        executor: impl TaskExecutor + Clone + Send + 'static,
    ) -> Swarm<NimiqBehaviour> {
        let local_peer_id = PeerId::from(config.keypair.public());

        let transport = Self::new_transport(&config.keypair, config.memory_transport).unwrap();

        let behaviour = NimiqBehaviour::new(config, clock, contacts, peer_score_params);

        let limits = ConnectionLimits::default()
            .with_max_pending_incoming(Some(16))
            .with_max_pending_outgoing(Some(16))
            .with_max_established_incoming(Some(4800))
            .with_max_established_outgoing(Some(4800))
            .with_max_established_per_peer(Some(MAX_CONNECTIONS_PER_PEER));

        // TODO add proper config
        SwarmBuilder::with_executor(
            transport,
            behaviour,
            local_peer_id,
            Box::new(move |fut| {
                executor.exec(fut);
            }),
        )
        .connection_limits(limits)
        .build()
    }

    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    #[cfg(feature = "tokio-time")]
    async fn swarm_task(
        mut swarm: NimiqSwarm,
        events_tx: broadcast::Sender<NetworkEvent<PeerId>>,
        mut action_rx: mpsc::Receiver<NetworkAction>,
        mut validate_rx: mpsc::UnboundedReceiver<ValidateMessage<PeerId>>,
        connected_peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
        peer_request_limits: Arc<Mutex<HashMap<PeerId, HashMap<u16, RateLimit>>>>,
        rate_limits_pending_deletion: Arc<Mutex<PendingDeletion>>,
        mut update_scores: Interval,
        contacts: Arc<RwLock<PeerContactBook>>,
        #[cfg(feature = "metrics")] metrics: Arc<NetworkMetrics>,
    ) {
        let mut task_state = TaskState::default();

        let peer_id = Swarm::local_peer_id(&swarm);
        let task_span = trace_span!("swarm task", peer_id=?peer_id);

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
                                Ok(false) => debug!(topic, "Validation took too long: message is no longer in the message cache"),
                                Err(e) => error!(topic, error = %e, "Network error while relaying message"),
                            }
                        }
                    },
                    event = swarm.next() => {
                        if let Some(event) = event {
                            Self::handle_event(event, &events_tx, &mut swarm, &mut task_state, &connected_peers, Arc::clone(&peer_request_limits), Arc::clone(&rate_limits_pending_deletion), #[cfg( feature = "metrics")] &metrics);
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
                    _ = update_scores.tick() => {
                        swarm.behaviour().update_scores(Arc::clone(&contacts));
                    },
                };
            }
        }
        .instrument(task_span)
        .await
    }

    // This is a duplicate of the previous function.
    // This is because these two functions use different implementation for the handling of intervals,
    // And the tokio version (needed for some test) could  not be reconciled  with the non tokio version
    // Essentially the tokio select macro is not compatible with the condition compilation flag
    #[cfg(not(feature = "tokio-time"))]
    async fn swarm_task(
        mut swarm: NimiqSwarm,
        events_tx: broadcast::Sender<NetworkEvent<PeerId>>,
        mut action_rx: mpsc::Receiver<NetworkAction>,
        mut validate_rx: mpsc::UnboundedReceiver<ValidateMessage<PeerId>>,
        connected_peers: Arc<RwLock<HashMap<PeerId, PeerInfo>>>,
        peer_request_limits: Arc<Mutex<HashMap<PeerId, HashMap<u16, RateLimit>>>>,
        rate_limits_pending_deletion: Arc<Mutex<PendingDeletion>>,
        mut update_scores: Interval,
        contacts: Arc<RwLock<PeerContactBook>>,
        #[cfg(feature = "metrics")] metrics: Arc<NetworkMetrics>,
    ) {
        let mut task_state = TaskState::default();

        let peer_id = Swarm::local_peer_id(&swarm);
        let task_span = trace_span!("swarm task", peer_id=?peer_id);

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
                                Ok(false) => debug!(topic, "Validation took too long: message is no longer in the message cache"),
                                Err(e) => error!(topic, error = %e, "Network error while relaying message"),
                            }
                        }
                    },
                    event = swarm.next() => {
                        if let Some(event) = event {
                            Self::handle_event(event, &events_tx, &mut swarm, &mut task_state, &connected_peers, Arc::clone(&peer_request_limits), Arc::clone(&rate_limits_pending_deletion), #[cfg( feature = "metrics")] &metrics);
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
                    _ = update_scores.next() => {
                        swarm.behaviour().update_scores(Arc::clone(&contacts));
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
        connected_peers: &RwLock<HashMap<PeerId, PeerInfo>>,
        peer_request_limits: Arc<Mutex<HashMap<PeerId, HashMap<u16, RateLimit>>>>,
        rate_limits_pending_deletion: Arc<Mutex<PendingDeletion>>,
        #[cfg(feature = "metrics")] metrics: &Arc<NetworkMetrics>,
    ) {
        match event {
            SwarmEvent::ConnectionEstablished {
                peer_id,
                endpoint,
                num_established,
                concurrent_dial_errors,
            } => {
                debug!(
                    %peer_id,
                    ?endpoint,
                    connections = num_established,
                    "Connection established",
                );

                if let Some(dial_errors) = concurrent_dial_errors {
                    for (addr, error) in dial_errors {
                        debug!(
                            %peer_id,
                            address = %addr,
                            %error,
                            "Removing addresses that caused dial failures",
                        );
                        swarm.behaviour_mut().remove_peer_address(peer_id, addr);
                    }
                }

                // Save dialed peer addresses
                if endpoint.is_dialer() {
                    let listen_addr = endpoint.get_remote_address();

                    debug!(%peer_id, address = %listen_addr, "Saving peer");

                    swarm
                        .behaviour_mut()
                        .add_peer_address(peer_id, listen_addr.clone());

                    // Bootstrap Kademlia if we're performing our first connection
                    if !state.is_bootstrapped {
                        debug!("Bootstrapping DHT");
                        if swarm.behaviour_mut().dht.bootstrap().is_err() {
                            error!("Bootstrapping DHT error: No known peers");
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
                info!(
                    %peer_id,
                    ?endpoint,
                    connections = num_established,
                    "Connection closed with peer",
                );

                if let Some(cause) = cause {
                    info!(%cause, "Connection closed because");
                }

                // Remove Peer
                if num_established == 0 {
                    connected_peers.write().remove(&peer_id);
                    swarm.behaviour_mut().remove_peer(peer_id);

                    // Removes or marks to remove the respective rate limits.
                    // Also cleans up the expired rate limits pending to delete.
                    Self::remove_rate_limits(
                        peer_request_limits,
                        rate_limits_pending_deletion,
                        peer_id,
                    );

                    if let Err(error) = events_tx.send(NetworkEvent::PeerLeft(peer_id)) {
                        error!(%error, "could not send peer left event to channel");
                    }
                }
            }

            SwarmEvent::IncomingConnection {
                local_addr,
                send_back_addr,
            } => {
                debug!(
                    address = %send_back_addr,
                    listen_address = %local_addr,
                    "Incoming connection",
                );
            }

            SwarmEvent::IncomingConnectionError {
                local_addr,
                send_back_addr,
                error,
            } => {
                debug!(
                    address = %send_back_addr,
                    listen_address = %local_addr,
                    %error,
                    "Incoming connection error",
                );
            }

            SwarmEvent::Dialing(peer_id) => {
                // This event is only triggered if the network behaviour performs the dial
                debug!(%peer_id, "Dialing peer");
            }

            SwarmEvent::Behaviour(event) => {
                match event {
                    NimiqEvent::Dht(event) => {
                        match event {
                            KademliaEvent::OutboundQueryProgressed { id, result, .. } => {
                                match result {
                                    QueryResult::GetRecord(result) => {
                                        if let Some(output) = state.dht_gets.remove(&id) {
                                            let result = result
                                                .map_err(Into::into)
                                                .map(|record| match record {
                                                    GetRecordOk::FoundRecord(r) => Some(r.record.value),
                                                    GetRecordOk::FinishedWithNoAdditionalRecord { cache_candidates: _ } => None,
                                                });
                                            if output.send(result).is_err() {
                                                error!(query_id = ?id, error = "receiver hung up", "could not send get record query result to channel");
                                            }
                                        } else {
                                            warn!(query_id = ?id, "GetRecord query result for unknown query ID");
                                        }
                                    }
                                    QueryResult::PutRecord(result) => {
                                        // dht_put resolved
                                        if let Some(output) = state.dht_puts.remove(&id) {
                                            if output
                                                .send(result.map(|_| ()).map_err(Into::into))
                                                .is_err()
                                            {
                                                error!(query_id = ?id, error = "receiver hung up", "could not send put record query result to channel");
                                            }
                                        } else {
                                            warn!(query_id = ?id, "PutRecord query result for unknown query ID");
                                        }
                                    }
                                    QueryResult::Bootstrap(result) => match result {
                                        Ok(result) => {
                                            debug!(?result, "DHT bootstrap successful")
                                        }
                                        Err(e) => error!(error = %e, "DHT bootstrap error"),
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
                                    // TODO: Move uncompress to caller side
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
                                                    error!("Could not store record in DHT record store");
                                                    return;
                                                };
                                            } else {
                                                warn!(public_key = %pk, "DHT record signature verification failed. Record public key");
                                                return;
                                            }
                                        }
                                    }
                                }
                                warn!(
                                    "DHT record verification failed: Invalid public key received"
                                );
                            }
                            _ => {}
                        }
                    }
                    NimiqEvent::Discovery(event) => {
                        swarm.behaviour_mut().pool.maintain_peers();
                        match event {
                            DiscoveryEvent::Established {
                                peer_id,
                                peer_address,
                                peer_contact,
                            } => {
                                let peer_info = PeerInfo::new(peer_address, peer_contact.services);
                                if connected_peers
                                    .write()
                                    .insert(peer_id, peer_info.clone())
                                    .is_none()
                                {
                                    info!(%peer_id, "Peer joined");
                                    if let Err(error) =
                                        events_tx.send(NetworkEvent::PeerJoined(peer_id, peer_info))
                                    {
                                        error!(%peer_id, %error, "could not send peer joined event to channel");
                                    }
                                } else {
                                    error!(%peer_id, "Peer joined but it already exists");
                                }
                            }
                            DiscoveryEvent::Update => {}
                        }
                    }
                    NimiqEvent::Gossip(event) => match event {
                        GossipsubEvent::Message {
                            propagation_source,
                            message_id,
                            message,
                        } => {
                            let topic = message.topic.clone();
                            if let Some(topic_info) = state.gossip_topics.get_mut(&topic) {
                                let (output, validate) = topic_info;
                                if !&*validate {
                                    if let Err(error) = swarm
                                        .behaviour_mut()
                                        .gossipsub
                                        .report_message_validation_result(
                                            &message_id,
                                            &propagation_source,
                                            MessageAcceptance::Accept,
                                        )
                                    {
                                        error!(%message_id, %error, "could not send message validation result to channel");
                                    }
                                }

                                if let Err(error) =
                                    output.try_send((message, message_id, propagation_source))
                                {
                                    error!(
                                        %topic,
                                        %error,
                                        "Failed to dispatch gossipsub message",
                                    )
                                }
                            } else {
                                warn!(topic = %message.topic, "unknown topic hash");
                            }
                            #[cfg(feature = "metrics")]
                            metrics.note_received_pubsub_message(&topic);
                        }
                        GossipsubEvent::Subscribed { peer_id, topic } => {
                            debug!(%peer_id, %topic, "peer subscribed to topic");
                        }
                        GossipsubEvent::Unsubscribed { peer_id, topic } => {
                            debug!(%peer_id, %topic, "peer unsubscribed");
                        }
                        GossipsubEvent::GossipsubNotSupported { peer_id } => {
                            debug!(%peer_id, "gossipsub not supported");
                        }
                    },
                    NimiqEvent::Identify(event) => {
                        match event {
                            IdentifyEvent::Received { peer_id, info } => {
                                debug!(
                                    %peer_id,
                                    address = %info.observed_addr,
                                    info = ?info,
                                    "Received identity",
                                );

                                // Save identified peer listen addresses
                                for listen_addr in info.listen_addrs {
                                    swarm.behaviour_mut().add_peer_address(peer_id, listen_addr);

                                    // Bootstrap Kademlia if we're adding our first address
                                    if !state.is_bootstrapped {
                                        debug!("Bootstrapping DHT");
                                        if swarm.behaviour_mut().dht.bootstrap().is_err() {
                                            error!("Bootstrapping DHT error: No known peers");
                                        }
                                        state.is_bootstrapped = true;
                                    }
                                }
                            }
                            IdentifyEvent::Pushed { peer_id } => {
                                trace!(%peer_id, "Pushed identity to peer");
                            }
                            IdentifyEvent::Sent { peer_id } => {
                                trace!(%peer_id, "Sent identity to peer");
                            }
                            IdentifyEvent::Error { peer_id, error } => {
                                error!(
                                    %peer_id,
                                    %error,
                                    "Error while identifying remote peer",
                                );
                            }
                        }
                    }
                    NimiqEvent::Ping(event) => {
                        match event.result {
                            Err(error) => {
                                log::debug!(%error, ?event.peer, "Ping failed with peer");
                            }
                            Ok(PingSuccess::Pong) => {
                                log::trace!(?event.peer, "Responded Ping from peer");
                            }
                            Ok(PingSuccess::Ping { rtt }) => {
                                log::trace!(
                                    ?event.peer,
                                    ?rtt,
                                    "Sent Ping and received response to/from peer",
                                );
                            }
                        };
                    }
                    NimiqEvent::Pool(event) => {
                        match event {
                            ConnectionPoolEvent::PeerJoined { peer_id: _ } => {}
                        };
                    }
                    NimiqEvent::RequestResponse(event) => match event {
                        RequestResponseEvent::Message {
                            peer: peer_id,
                            message,
                        } => match message {
                            RequestResponseMessage::Request {
                                request_id,
                                request,
                                channel,
                            } => {
                                // TODO Add rate limiting (per peer).
                                if let Ok(type_id) = peek_type(&request) {
                                    trace!(
                                        %request_id,
                                        %peer_id,
                                        %type_id,
                                        content = &*base64::prelude::BASE64_STANDARD.encode(&request),
                                        "Incoming request from peer",
                                    );
                                    // Check if we have a receiver registered for this message type
                                    let sender = match state.receive_requests.get_mut(&type_id) {
                                        // Check if the sender is still alive, if not remove it
                                        Some(sender) if !sender.is_closed() => Some(sender),
                                        Some(_) => {
                                            state.receive_requests.remove(&type_id);
                                            None
                                        }
                                        None => None,
                                    };
                                    // If we have a receiver, pass the request. Otherwise send a default empty response
                                    if let Some(sender) = sender {
                                        if type_id.requires_response() {
                                            state.response_channels.insert(request_id, channel);
                                        } else {
                                            // Respond on behalf of the actual
                                            // receiver because the actual
                                            // receiver isn't interested in
                                            // responding.
                                            let response: Result<(), InboundRequestError> = Ok(());
                                            if swarm
                                                .behaviour_mut()
                                                .request_response
                                                .send_response(channel, response.serialize_to_vec())
                                                .is_err()
                                            {
                                                error!(
                                                    %request_id,
                                                    %peer_id,
                                                    %type_id,
                                                    "Could not send auto response",
                                                );
                                            }
                                        }
                                        if let Err(e) =
                                            sender.try_send((request.into(), request_id, peer_id))
                                        {
                                            error!(
                                                %request_id,
                                                %peer_id,
                                                %type_id,
                                                error = %e,
                                                "Failed to dispatch request from peer",
                                            );
                                        }
                                    } else {
                                        trace!(
                                            %request_id,
                                            %peer_id,
                                            %type_id,
                                            "No receiver found for requests of this type, replying with a 'NoReceiver' error",
                                        );
                                        let err: Result<(), InboundRequestError> =
                                            Err(InboundRequestError::NoReceiver);
                                        if swarm
                                            .behaviour_mut()
                                            .request_response
                                            .send_response(channel, err.serialize_to_vec())
                                            .is_err()
                                        {
                                            error!(
                                                %request_id,
                                                %peer_id,
                                                %type_id,
                                                "Could not send default response",
                                            );
                                        };
                                    }
                                } else {
                                    error!(
                                        %request_id,
                                        %peer_id,
                                        content = &*base64::prelude::BASE64_STANDARD.encode(&request),
                                        "Could not parse request type",
                                    );
                                }
                            }
                            RequestResponseMessage::Response {
                                request_id,
                                response,
                            } => {
                                trace!(
                                    %request_id,
                                    %peer_id,
                                    "Incoming response from peer",
                                );
                                if let Some(channel) = state.requests.remove(&request_id) {
                                    if channel.send(Ok(response.into())).is_err() {
                                        error!(%request_id, %peer_id, error = "receiver hung up", "could not send response to channel");
                                    }

                                    #[cfg(feature = "metrics")]
                                    if let Some(instant) =
                                        state.requests_initiated.remove(&request_id)
                                    {
                                        metrics.note_response_time(instant.elapsed());
                                    }
                                } else {
                                    error!(
                                        %request_id,
                                        "No such request ID found",
                                    );
                                }
                            }
                        },
                        RequestResponseEvent::OutboundFailure {
                            peer: peer_id,
                            request_id,
                            error,
                        } => {
                            error!(
                                %request_id,
                                %peer_id,
                                %error,
                                "Failed to send request to peer",
                            );
                            if let Some(channel) = state.requests.remove(&request_id) {
                                if channel.send(Err(Self::to_response_error(error))).is_err() {
                                    error!(%request_id, %peer_id, error = "receiver hung up", "could not send outbound failure to channel");
                                }
                            } else {
                                error!(
                                    %request_id,
                                    %peer_id,
                                    "No such request ID found"
                                );
                            }
                        }
                        RequestResponseEvent::InboundFailure {
                            peer,
                            request_id,
                            error,
                        } => {
                            error!(
                                %request_id,
                                peer_id = %peer,
                                %error,
                                "Response to request sent from peer failed",
                            );
                        }
                        RequestResponseEvent::ResponseSent { peer, request_id } => {
                            trace!(
                                %request_id,
                                peer_id = %peer,
                                "Response sent to peer",
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
        // trace!(?action, "performing action");

        match action {
            NetworkAction::Dial { peer_id, output } => {
                if output
                    .send(
                        Swarm::dial(swarm, DialOpts::peer_id(peer_id).build()).map_err(Into::into),
                    )
                    .is_err()
                {
                    error!(%peer_id, error = "receiver hung up", "could not send dial to channel");
                }
            }
            NetworkAction::DialAddress { address, output } => {
                if output
                    .send(
                        Swarm::dial(swarm, DialOpts::unknown_peer_id().address(address).build())
                            .map_err(Into::into),
                    )
                    .is_err()
                {
                    error!(error = "receiver hung up", "could not send dial to channel");
                }
            }
            NetworkAction::DhtGet { key, output } => {
                let query_id = swarm.behaviour_mut().dht.get_record(key.into());
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
                        if output.send(Err(e.into())).is_err() {
                            error!(
                                error = "receiver hung up",
                                "could not send dht put error to channel",
                            );
                        }
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
                            Ok(_) => {
                                if output.send(Ok(rx)).is_err() {
                                    error!(%topic_name, error = "receiver hung up", "could not send subscribe result to channel");
                                }
                            }
                            Err(e) => {
                                if output
                                    .send(Err(NetworkError::TopicScoreParams {
                                        topic_name,
                                        error: e,
                                    }))
                                    .is_err()
                                {
                                    error!(%topic_name, error = "receiver hung up", "could not send subscribe error to channel");
                                }
                            }
                        };
                    }

                    // Apparently we're already subscribed.
                    Ok(false) => {
                        if output
                            .send(Err(NetworkError::AlreadySubscribed { topic_name }))
                            .is_err()
                        {
                            error!(%topic_name, error = "receiver hung up", "could not send subscribe already subscribed error to channel");
                        }
                    }

                    // Subscribe failed. Send back error.
                    Err(e) => {
                        if output.send(Err(e.into())).is_err() {
                            error!(%topic_name, error = "receiver hung up", "could not send subscribe error2 to channel");
                        }
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
                            if output.send(Ok(())).is_err() {
                                error!(%topic_name, error = "receiver hung up", "could not send unsubscribe result to channel");
                            }
                        }

                        // Apparently we're already unsubscribed.
                        Ok(false) => {
                            drop(state.gossip_topics.remove(&topic.hash()).unwrap().0);
                            if output
                                .send(Err(NetworkError::AlreadyUnsubscribed { topic_name }))
                                .is_err()
                            {
                                error!(%topic_name, error = "receiver hung up", "could not send unsubscribe already unsubscribed error to channel");
                            }
                        }

                        // Unsubscribe failed. Send back error.
                        Err(e) => {
                            if output.send(Err(e.into())).is_err() {
                                error!(%topic_name, error = "receiver hung up", "could not send unsubscribe error to channel");
                            }
                        }
                    }
                } else {
                    // If the topic wasn't in the topics list, we're not subscribed to it.
                    if output
                        .send(Err(NetworkError::AlreadyUnsubscribed { topic_name }))
                        .is_err()
                    {
                        error!(%topic_name, error = "receiver hung up", "could not send unsubscribe already unsubscribed2 error to channel");
                    }
                }
            }
            NetworkAction::Publish {
                topic_name,
                data,
                output,
            } => {
                let topic = IdentTopic::new(topic_name);

                if output
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
                    .is_err()
                {
                    error!(%topic_name, error = "receiver hung up", "could not send publish result to channel");
                }
            }
            NetworkAction::NetworkInfo { output } => {
                if output.send(Swarm::network_info(swarm)).is_err() {
                    error!(
                        error = "receiver hung up",
                        "could not send network info result to channel",
                    );
                }
            }
            NetworkAction::ReceiveRequests { type_id, output } => {
                state.receive_requests.insert(type_id, output);
            }
            NetworkAction::SendRequest {
                peer_id,
                request,
                request_type_id,
                response_channel,
                output,
            } => {
                let request_id = swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer_id, request);
                trace!(
                    %request_id,
                    %peer_id,
                    type_id = %request_type_id,
                    "Request was sent to peer",
                );
                state.requests.insert(request_id, response_channel);
                #[cfg(feature = "metrics")]
                state.requests_initiated.insert(request_id, Instant::now());
                if output.send(request_id).is_err() {
                    error!(%peer_id, %request_type_id, error = "receiver hung up", "could not send send request result to channel");
                }
            }
            NetworkAction::SendResponse {
                request_id,
                response,
                output,
            } => {
                if let Some(response_channel) = state.response_channels.remove(&request_id) {
                    if output
                        .send(
                            swarm
                                .behaviour_mut()
                                .request_response
                                .send_response(response_channel, response)
                                .map_err(NetworkError::ResponseChannelClosed),
                        )
                        .is_err()
                    {
                        error!(
                            %request_id,
                            error = "receiver hung up",
                            "Response was sent but the action channel was dropped",
                        );
                    };
                } else {
                    error!(%request_id, "Tried to respond to a non existing request");
                    if output.send(Err(NetworkError::UnknownRequestId)).is_err() {
                        error!(%request_id, error = "receiver hung up", "could not send unknown request ID to channel");
                    }
                }
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
            NetworkAction::DisconnectPeer { peer_id } => {
                if swarm.disconnect_peer_id(peer_id).is_err() {
                    warn!(%peer_id, "Peer already closed");
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
        if let Err(error) = self
            .action_tx
            .clone()
            .send(NetworkAction::ListenOn { listen_addresses })
            .await
        {
            error!(%error, "Failed to send NetworkAction::ListenOnAddress");
        }
    }

    pub async fn start_connecting(&self) {
        if let Err(error) = self
            .action_tx
            .clone()
            .send(NetworkAction::StartConnecting)
            .await
        {
            error!(%error, "Failed to send NetworkAction::StartConnecting");
        }
    }

    async fn request_impl<Req: RequestCommon>(
        &self,
        request: Req,
        peer_id: PeerId,
    ) -> Result<Req::Response, RequestError> {
        let (output_tx, output_rx) = oneshot::channel();
        let (response_tx, response_rx) = oneshot::channel();

        let mut buf = vec![];
        if request.serialize_request(&mut buf).is_err() {
            return Err(RequestError::OutboundRequest(
                OutboundRequestError::SerializationError,
            ));
        }

        if self
            .action_tx
            .clone()
            .send(NetworkAction::SendRequest {
                peer_id,
                request: buf[..].into(),
                request_type_id: RequestType::from_request::<Req>(),
                response_channel: response_tx,
                output: output_tx,
            })
            .await
            .is_err()
        {
            return Err(RequestError::OutboundRequest(
                OutboundRequestError::SendError,
            ));
        }

        if let Ok(request_id) = output_rx.await {
            let result = response_rx.await;
            match result {
                Err(_) => Err(RequestError::OutboundRequest(
                    OutboundRequestError::SenderFutureDropped,
                )),
                Ok(result) => {
                    let data = result?;
                    if let Ok(message) =
                        <Result<Req::Response, InboundRequestError> as Deserialize>::deserialize(
                            &mut data.reader(),
                        )
                    {
                        // Check if there was an actual response from the application or a default response from
                        // the network. If the network replied with the default response, it was because there wasn't a
                        // receiver for the request
                        match message {
                            Ok(message) => Ok(message),
                            Err(e) => Err(RequestError::InboundRequest(e)),
                        }
                    } else {
                        error!(
                        %request_id,
                        %peer_id,
                        type_id = std::any::type_name::<Req::Response>(),
                        "Failed to deserialize response from peer",
                        );
                        Err(RequestError::InboundRequest(
                            InboundRequestError::DeSerializationError,
                        ))
                    }
                }
            }
        } else {
            Err(RequestError::OutboundRequest(
                OutboundRequestError::SendError,
            ))
        }
    }

    fn receive_requests_impl<Req: RequestCommon>(
        &self,
    ) -> BoxStream<'static, (Req, RequestId, PeerId)> {
        enum ReceiveStream {
            WaitingForRegister(
                Pin<Box<dyn Future<Output = mpsc::Receiver<(Bytes, RequestId, PeerId)>> + Send>>,
            ),
            Registered(mpsc::Receiver<(Bytes, RequestId, PeerId)>),
        }

        impl Stream for ReceiveStream {
            type Item = (Bytes, RequestId, PeerId);
            fn poll_next(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<Option<(Bytes, RequestId, PeerId)>> {
                let self_ = self.get_mut();
                loop {
                    use ReceiveStream::*;
                    match self_ {
                        WaitingForRegister(fut) => {
                            *self_ = Registered(ready!(Pin::new(fut).poll(cx)))
                        }
                        Registered(stream) => return stream.poll_recv(cx),
                    }
                }
            }
            // The default size_hint is the best we can do, we can never say if
            // there are going to be more requests or none at all.
        }

        let peer_request_limits = Arc::clone(&self.peer_request_limits);
        let action_tx = self.action_tx.clone();
        let action_tx2 = self.action_tx.clone();
        ReceiveStream::WaitingForRegister(Box::pin(async move {
            // TODO Make buffer size configurable
            let (tx, rx) = mpsc::channel(1024);

            action_tx
                .send(NetworkAction::ReceiveRequests {
                    type_id: RequestType::from_request::<Req>(),
                    output: tx,
                })
                .await
                .expect("Sending action to network task failed.");

            rx
        }))
        .filter_map(move |(data, request_id, peer_id)| {
            let peer_request_limits = Arc::clone(&peer_request_limits);
            let action_tx2 = action_tx2.clone();
            async move {
                // If the request is not respecting the rate limits for its request type, filters the request out
                // and replies with the respective error message.
                if !Self::is_under_the_rate_limits::<Req>(peer_request_limits, peer_id, request_id)
                {
                    info!(
                        %request_id,
                        %peer_id,
                        type_id = std::any::type_name::<Req>(),
                        "Rate limit was exceeded!",
                    );
                    if let Err(e) = Self::respond_with_error::<Req>(
                        action_tx2,
                        request_id,
                        InboundRequestError::ExceedsRateLimit,
                    )
                    .await
                    {
                        trace!(
                            "Error while sending a Exceeds Rate limit error to the sender {:?}",
                            e
                        );
                    }
                    return None;
                }

                // Map the (data, peer) stream to (message, peer) by deserializing the messages.
                match Req::deserialize_request(&mut data.reader()) {
                    Ok(message) => Some((message, request_id, peer_id)),
                    Err(e) => {
                        error!(
                            %request_id,
                            %peer_id,
                            type_id = std::any::type_name::<Req>(),
                            error = %e,
                            "Failed to deserialize request from peer",
                        );
                        None
                    }
                }
            }
        })
        .boxed()
    }

    fn to_response_error(error: OutboundFailure) -> RequestError {
        match error {
            OutboundFailure::ConnectionClosed => {
                RequestError::OutboundRequest(OutboundRequestError::ConnectionClosed)
            }
            OutboundFailure::DialFailure => {
                RequestError::OutboundRequest(OutboundRequestError::DialFailure)
            }
            OutboundFailure::Timeout => {
                RequestError::OutboundRequest(OutboundRequestError::Timeout)
            }
            OutboundFailure::UnsupportedProtocols => {
                RequestError::OutboundRequest(OutboundRequestError::UnsupportedProtocols)
            }
        }
    }

    pub fn peer_count(&self) -> usize {
        self.connected_peers.read().len()
    }

    pub async fn disconnect(&self) {
        for peer_id in self.get_peers() {
            self.disconnect_peer(peer_id, CloseReason::Other).await;
        }
    }

    #[cfg(feature = "metrics")]
    pub fn metrics(&self) -> Arc<NetworkMetrics> {
        self.metrics.clone()
    }

    fn is_under_the_rate_limits<Req: RequestCommon>(
        peer_request_limits: Arc<Mutex<HashMap<PeerId, HashMap<u16, RateLimit>>>>,
        peer_id: PeerId,
        request_id: RequestId,
    ) -> bool {
        // Gets lock of peer requests limits read and write on it.
        let mut peer_request_limits = peer_request_limits.lock();

        // If the peer has never sent a request of this type, creates a new entry.
        let requests_limit = peer_request_limits
            .entry(peer_id)
            .or_default()
            .entry(Req::TYPE_ID)
            .or_insert_with(|| RateLimit::new(Req::MAX_REQUESTS, Req::TIME_WINDOW, Instant::now()));

        // Ensures that the request is allowed based on the set limits and updates the counter.
        // Returns early if not allowed.
        if !requests_limit.increment_and_is_allowed(1) {
            log::debug!(
                "[{:?}][{:?}] {:?} Exceeded max requests rate {:?} requests per {:?} seconds",
                request_id,
                peer_id,
                std::any::type_name::<Req>(),
                Req::MAX_REQUESTS,
                Req::TIME_WINDOW,
            );
            return false;
        }
        true
    }

    fn remove_rate_limits(
        peer_request_limits: Arc<Mutex<HashMap<PeerId, HashMap<u16, RateLimit>>>>,
        rate_limits_pending_deletion: Arc<Mutex<PendingDeletion>>,
        peer_id: PeerId,
    ) {
        // Every time a peer disconnects, we delete all expired pending limits.
        Self::clean_up(
            Arc::clone(&peer_request_limits),
            Arc::clone(&rate_limits_pending_deletion),
        );

        // Firstly we must acquire the lock of the pending deletes to avoid deadlocks.
        let mut rate_limits_pending_deletion_l = rate_limits_pending_deletion.lock();
        let mut peer_request_limits_l = peer_request_limits.lock();

        // Go through all existing request types of the given peer and deletes the limit counters if possible or marks it for deletion.
        if let Some(request_limits) = peer_request_limits_l.get_mut(&peer_id) {
            request_limits.retain(|req_type, rate_limit| {
                // Gets the requests limit and deletes it if no counter info would be lost, otherwise places it as pending deletion.
                if !rate_limit.can_delete(Instant::now()) {
                    rate_limits_pending_deletion_l.insert(peer_id, *req_type, rate_limit);
                    true
                } else {
                    false
                }
            });
            // If the peer no longer has any pending rate limits, then it gets removed.
            if request_limits.is_empty() {
                peer_request_limits_l.remove(&peer_id);
            }
        }
    }

    /// Deletes the rate limits that were previously marked as pending if its expiration time has passed.
    fn clean_up(
        peer_request_limits: Arc<Mutex<HashMap<PeerId, HashMap<u16, RateLimit>>>>,
        rate_limits_pending_deletion: Arc<Mutex<PendingDeletion>>,
    ) {
        let mut rate_limits_pending_deletion_l = rate_limits_pending_deletion.lock();

        // Iterates from the oldest to the most recent expiration date and deletes the entries that have expired.
        // The pending to deletion is ordered from the oldest to the most recent expiration date, thus we break early
        // from the loop once we find a non expired rate limit.
        while let Some(peer_expiration) = rate_limits_pending_deletion_l.first() {
            let current_timestamp = Instant::now();
            if peer_expiration.expiration_time <= current_timestamp {
                let mut peer_request_limits_l = peer_request_limits.lock();

                if let Some(peer_req_limits) = peer_request_limits_l
                    .get_mut(&peer_expiration.peer_id)
                    .and_then(|peer_req_limits| {
                        if let Some(rate_limit) = peer_req_limits.get(&peer_expiration.req_type) {
                            // If the peer has reconnected the rate limit may be enforcing a new limit. In this case we only remove
                            // the pending deletion.
                            if rate_limit.can_delete(current_timestamp) {
                                peer_req_limits.remove(&peer_expiration.req_type);
                            }
                            return Some(peer_req_limits);
                        }
                        // Only returns None if no request type was found.
                        None
                    })
                {
                    // If the peer no longer has any pending rate limits, then it gets removed from both rate limits and pending deletion.
                    if peer_req_limits.is_empty() {
                        peer_request_limits_l.remove(&peer_expiration.peer_id);
                    }
                } else {
                    // If the information is in pending deletion, that should mean it was not deleted from peer_request_limits yet, so that
                    // reconnection doesn't bypass the limits we are enforcing.
                    unreachable!(
                        "Tried to remove a non existing rate limit from peer_request_limits."
                    );
                }
                // Removes the entry from the pending for deletion.
                rate_limits_pending_deletion_l.remove_first();
            } else {
                break;
            }
        }
    }

    async fn respond_with_error<Req: RequestCommon>(
        action_tx: mpsc::Sender<NetworkAction>,
        request_id: RequestId,
        response: InboundRequestError,
    ) -> Result<(), NetworkError> {
        let (output_tx, output_rx) = oneshot::channel();

        // Encapsulate it in a `Result` to signal the network that this
        // was a unsuccessful response from the application.
        let response: Result<Req::Response, InboundRequestError> = Err(response);
        let buf = response.serialize_to_vec();

        action_tx
            .clone()
            .send(NetworkAction::SendResponse {
                request_id,
                response: buf,
                output: output_tx,
            })
            .await?;

        output_rx.await?
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

    fn get_peer_info(&self, peer_id: Self::PeerId) -> Option<PeerInfo> {
        self.connected_peers.read().get(&peer_id).cloned()
    }

    fn peer_provides_required_services(&self, peer_id: PeerId) -> bool {
        if let Some(peer_info) = self.connected_peers.read().get(&peer_id) {
            peer_info.get_services().contains(self.required_services)
        } else {
            // If we don't know the peer we return false
            false
        }
    }

    async fn disconnect_peer(&self, peer_id: PeerId, _close_reason: CloseReason) {
        if let Err(error) = self
            .action_tx
            .send(NetworkAction::DisconnectPeer { peer_id })
            .await
        {
            error!(%peer_id, %error, "could not send disconnect action to channel");
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

        #[cfg(feature = "metrics")]
        self.metrics
            .note_published_pubsub_message(<T as Topic>::NAME);

        Ok(())
    }

    fn validate_message<T>(&self, pubsub_id: Self::PubsubId, acceptance: MsgAcceptance)
    where
        T: Topic + Sync,
    {
        self.validate_tx
            .send(ValidateMessage::new::<T>(pubsub_id, acceptance))
            .ok()
            .expect("Failed to send reported message validation result: receiver hung up");
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

    async fn message<M: Message>(&self, message: M, peer_id: PeerId) -> Result<(), RequestError> {
        self.request_impl(message, peer_id).await
    }

    async fn request<Req: Request>(
        &self,
        request: Req,
        peer_id: PeerId,
    ) -> Result<Req::Response, RequestError> {
        self.request_impl(request, peer_id).await
    }

    fn receive_messages<M: Message>(&self) -> BoxStream<'static, (M, PeerId)> {
        self.receive_requests_impl()
            .map(|(request, _, sender)| (request, sender))
            .boxed()
    }

    fn receive_requests<Req: Request>(&self) -> BoxStream<'static, (Req, RequestId, PeerId)> {
        self.receive_requests_impl()
    }

    async fn respond<Req: Request>(
        &self,
        request_id: RequestId,
        response: Req::Response,
    ) -> Result<(), Self::Error> {
        let (output_tx, output_rx) = oneshot::channel();

        let mut buf = vec![];

        // Encapsulate it in a `Result` to signal the network that this
        // was a successful response from the application.
        let response: Result<Req::Response, InboundRequestError> = Ok(response);
        response.serialize(&mut buf)?;

        self.action_tx
            .clone()
            .send(NetworkAction::SendResponse {
                request_id,
                response: buf,
                output: output_tx,
            })
            .await?;

        output_rx.await?
    }
}
