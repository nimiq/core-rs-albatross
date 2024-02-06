use std::{
    collections::HashMap,
    future::Future,
    num::NonZeroU8,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use async_trait::async_trait;
use base64::Engine;
use bytes::Bytes;
use futures::{future::BoxFuture, ready, stream::BoxStream, Stream, StreamExt};
#[cfg(not(feature = "tokio-time"))]
use instant::Instant;
#[cfg(all(target_family = "wasm", not(feature = "tokio-websocket")))]
use libp2p::websocket_websys;
use libp2p::{
    autonat, core,
    core::{
        muxing::StreamMuxerBox,
        transport::{Boxed, MemoryTransport},
    },
    gossipsub,
    identity::Keypair,
    kad::{
        self, store::RecordStore, GetRecordOk, InboundRequest, QueryId, QueryResult, Quorum, Record,
    },
    noise,
    request_response::{
        self, InboundRequestId, OutboundFailure, OutboundRequestId, ResponseChannel,
    },
    swarm::{
        dial_opts::{DialOpts, PeerCondition},
        NetworkInfo, SwarmEvent,
    },
    yamux, Multiaddr, PeerId, Swarm, SwarmBuilder, Transport,
};
#[cfg(feature = "tokio-websocket")]
use libp2p::{dns, tcp, websocket};
use log::Instrument;
use nimiq_bls::{CompressedPublicKey, KeyPair};
use nimiq_network_interface::{
    network::{
        CloseReason, MsgAcceptance, Network as NetworkInterface, NetworkEvent, PubsubId,
        SubscribeEvents, Topic,
    },
    peer_info::{PeerInfo, Services},
    request::{
        peek_type, InboundRequestError, Message, OutboundRequestError, Request, RequestCommon,
        RequestError, RequestSerialize, RequestType,
    },
};
use nimiq_primitives::task_executor::TaskExecutor;
use nimiq_serde::{Deserialize, DeserializeError, Serialize};
use nimiq_utils::tagged_signing::{TaggedKeyPair, TaggedSignable, TaggedSigned};
use nimiq_validator_network::validator_record::ValidatorRecord;
use parking_lot::{Mutex, RwLock};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot};
#[cfg(feature = "tokio-time")]
use tokio::time::{Instant, Interval};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
#[cfg(not(feature = "tokio-time"))]
use wasm_timer::Interval;

#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::{
    behaviour, connection_pool,
    discovery::{behaviour::Event, peer_contacts::PeerContactBook},
    dispatch::codecs::{IncomingRequest, OutgoingResponse},
    rate_limiting::{PendingDeletion, RateLimit},
    Config, NetworkError, TlsConfig,
};

type NimiqSwarm = Swarm<behaviour::Behaviour>;

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
        output: oneshot::Sender<Result<Vec<u8>, NetworkError>>,
    },
    DhtPut {
        key: Vec<u8>,
        value: Vec<u8>,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    Subscribe {
        topic_name: String,
        buffer_size: usize,
        validate: bool,
        output: oneshot::Sender<
            Result<
                mpsc::Receiver<(gossipsub::Message, gossipsub::MessageId, PeerId)>,
                NetworkError,
            >,
        >,
    },
    Unsubscribe {
        topic_name: String,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    Publish {
        topic_name: String,
        data: Vec<u8>,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    NetworkInfo {
        output: oneshot::Sender<NetworkInfo>,
    },
    ReceiveRequests {
        type_id: RequestType,
        output: mpsc::Sender<(Bytes, InboundRequestId, PeerId)>,
    },
    SendRequest {
        peer_id: PeerId,
        request: IncomingRequest,
        request_type_id: RequestType,
        response_channel: oneshot::Sender<Result<Bytes, RequestError>>,
        output: oneshot::Sender<OutboundRequestId>,
    },
    SendResponse {
        request_id: InboundRequestId,
        response: OutgoingResponse,
        output: oneshot::Sender<Result<(), NetworkError>>,
    },
    ListenOn {
        listen_addresses: Vec<Multiaddr>,
    },
    ConnectPeersByServices {
        services: Services,
        num_peers: usize,
        output: oneshot::Sender<Vec<PeerId>>,
    },
    StartConnecting,
    DisconnectPeer {
        peer_id: PeerId,
        reason: CloseReason,
    },
    UnbanPeer {
        peer_id: PeerId,
    },
}

struct ValidateMessage<P: Clone> {
    pubsub_id: GossipsubId<P>,
    acceptance: gossipsub::MessageAcceptance,
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
                MsgAcceptance::Accept => gossipsub::MessageAcceptance::Accept,
                MsgAcceptance::Ignore => gossipsub::MessageAcceptance::Ignore,
                MsgAcceptance::Reject => gossipsub::MessageAcceptance::Reject,
            },
            topic: <T as Topic>::NAME,
        }
    }
}

/// DHT bootstrap state
#[derive(Default, PartialEq)]
enum DhtBootStrapState {
    /// DHT bootstrap has been started
    #[default]
    NotStarted,
    /// DHT bootstrap has been started
    Started,
    /// DHT bootstrap has been completed
    Completed,
}

/// Enum over all of the possible DHT records values
#[derive(Clone, PartialEq)]
enum DhtRecord {
    /// Validator record with its publisher Peer ID,
    /// the decoded validator record and the original serialized record.
    Validator(PeerId, ValidatorRecord<PeerId>, Record),
}

impl DhtRecord {
    fn get_signed_record(self) -> Record {
        match self {
            Self::Validator(_, _, signed_record) => signed_record,
        }
    }

    fn get_peer_id(&self) -> PeerId {
        match self {
            Self::Validator(peer_id, _, _) => *peer_id,
        }
    }

    fn get_timestamp(&self) -> u64 {
        match self {
            Self::Validator(_, record, _) => record.timestamp,
        }
    }
}

impl PartialOrd for DhtRecord {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.get_timestamp().partial_cmp(&other.get_timestamp())
    }
}

/// DHT record decoding errors
#[derive(Debug, Error)]
enum DhtRecordError {
    /// Tag is unknown
    #[error("Unknown record tag")]
    UnknownTag,
    /// Deserialization error
    #[error("Deserialization error: {0}")]
    DeserializeError(#[from] DeserializeError),
}

impl TryFrom<&Record> for DhtRecord {
    type Error = DhtRecordError;
    fn try_from(record: &Record) -> Result<Self, Self::Error> {
        if let Some(tag) = TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::peek_tag(&record.value)
        {
            match tag {
                ValidatorRecord::<PeerId>::TAG => {
                    let validator_record =
                        TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::deserialize_from_vec(
                            &record.value,
                        )?;
                    {
                        Ok(DhtRecord::Validator(
                            record.publisher.unwrap(),
                            validator_record.record,
                            record.clone(),
                        ))
                    }
                }
                _ => Err(DhtRecordError::UnknownTag),
            }
        } else {
            Err(DhtRecordError::UnknownTag)
        }
    }
}

/// DHT results obtained for a specific query ID
struct DhtResults {
    /// Number of records obtained
    count: u8,
    /// Best value obtained so far
    best_value: DhtRecord,
    /// Other (outdated) values obtained
    outdated_values: Vec<DhtRecord>,
}

#[derive(Default)]
struct TaskState {
    /// Senders for DHT (kad) put operations
    dht_puts: HashMap<QueryId, oneshot::Sender<Result<(), NetworkError>>>,
    /// Senders for DHT (kad) get operations
    dht_gets: HashMap<QueryId, oneshot::Sender<Result<Vec<u8>, NetworkError>>>,
    /// Get results for DHT (kad) get operation
    dht_get_results: HashMap<QueryId, DhtResults>,
    /// Senders per Gossibsub topic
    gossip_topics: HashMap<
        gossipsub::TopicHash,
        (
            mpsc::Sender<(gossipsub::Message, gossipsub::MessageId, PeerId)>,
            bool,
        ),
    >,
    /// DHT (kad) has been bootstrapped
    dht_bootstrap_state: DhtBootStrapState,
    /// DHT (kad) is in server mode
    dht_server_mode: bool,
    /// Senders per `OutboundRequestId` for request-response
    requests: HashMap<OutboundRequestId, oneshot::Sender<Result<Bytes, RequestError>>>,
    /// Time spent per `OutboundRequestId` for request-response
    #[cfg(feature = "metrics")]
    requests_initiated: HashMap<OutboundRequestId, Instant>,
    /// Senders for receiving responses per `InboundRequestId` for request-response
    response_channels: HashMap<InboundRequestId, ResponseChannel<Option<OutgoingResponse>>>,
    /// Senders for replying to requests per `RequestType` for request-response
    receive_requests: HashMap<RequestType, mpsc::Sender<(Bytes, InboundRequestId, PeerId)>>,
    /// DHT quorum value
    dht_quorum: u8,
}

#[derive(Clone, Debug)]
pub struct GossipsubId<P: Clone> {
    message_id: gossipsub::MessageId,
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
    ///  - `config`: The network configuration, containing key pair, and other behavior-specific configuration.
    ///
    pub async fn new(config: Config, executor: impl TaskExecutor + Send + Clone + 'static) -> Self {
        let required_services = config.required_services;
        // TODO: persist to disk
        let own_peer_contact = config.peer_contact.clone();
        let contacts = Arc::new(RwLock::new(PeerContactBook::new(
            own_peer_contact.sign(&config.keypair),
            config.only_secure_ws_connections,
            config.allow_loopback_addresses,
            config.memory_transport,
        )));
        let params = gossipsub::PeerScoreParams {
            ip_colocation_factor_threshold: 20.0,
            ..Default::default()
        };
        let dht_quorum = config.dht_quorum;
        // Only force the server mode if we are doing a memory transport.
        // Otherwise expect the regular flow: DHT will get in server mode once a confirmed address is obtained using Autonat.
        // In memory transport we don't have a mechanism that sets the DHT in server mode such as confirming an address
        // with Autonat. This is because Autonat v1 only works with IP addresses.
        let force_dht_server_mode = config.memory_transport;
        let swarm = Self::new_swarm(
            config,
            Arc::clone(&contacts),
            params.clone(),
            force_dht_server_mode,
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
            force_dht_server_mode,
            dht_quorum,
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
        tls: Option<&TlsConfig>,
    ) -> std::io::Result<Boxed<(PeerId, StreamMuxerBox)>> {
        if memory_transport {
            // Memory transport primary for testing
            // TODO: Use websocket over the memory transport

            #[cfg(feature = "tokio-websocket")]
            let mut transport = websocket::WsConfig::new(dns::tokio::Transport::system(
                tcp::tokio::Transport::new(tcp::Config::default().nodelay(true)),
            )?);

            // Configure TLS if the configuration has the corresponding entry
            #[cfg(feature = "tokio-websocket")]
            if let Some(tls) = tls {
                let priv_key = websocket::tls::PrivateKey::new(tls.private_key.clone());
                let certificates: Vec<_> = tls
                    .certificates
                    .clone()
                    .into_iter()
                    .map(websocket::tls::Certificate::new)
                    .collect();
                transport
                    .set_tls_config(websocket::tls::Config::new(priv_key, certificates).unwrap());
            }
            #[cfg(not(feature = "tokio-websocket"))]
            let _ = tls; // silence unused variable warning

            #[cfg(feature = "tokio-websocket")]
            let transport = transport.or_transport(MemoryTransport::default());

            #[cfg(not(feature = "tokio-websocket"))]
            let transport = MemoryTransport::default();
            // Fixme: Handle wasm compatible transport

            let yamux = yamux::Config::default();

            Ok(transport
                .upgrade(core::upgrade::Version::V1)
                .authenticate(noise::Config::new(keypair).unwrap())
                .multiplex(yamux)
                .timeout(std::time::Duration::from_secs(20))
                .boxed())
        } else {
            #[cfg(feature = "tokio-websocket")]
            let mut transport = websocket::WsConfig::new(dns::tokio::Transport::system(
                tcp::tokio::Transport::new(tcp::Config::default().nodelay(true)),
            )?);

            // Configure TLS if the configuration has the corresponding entry
            #[cfg(feature = "tokio-websocket")]
            if let Some(tls) = tls {
                let priv_key = websocket::tls::PrivateKey::new(tls.private_key.clone());
                let certificates: Vec<_> = tls
                    .certificates
                    .clone()
                    .into_iter()
                    .map(websocket::tls::Certificate::new)
                    .collect();
                transport
                    .set_tls_config(websocket::tls::Config::new(priv_key, certificates).unwrap());
            }

            #[cfg(all(target_family = "wasm", not(feature = "tokio-websocket")))]
            let transport = websocket_websys::Transport::default();

            #[cfg(all(not(feature = "tokio-websocket"), not(target_family = "wasm")))]
            let transport = MemoryTransport::default();

            let yamux = yamux::Config::default();

            Ok(transport
                .upgrade(core::upgrade::Version::V1)
                .authenticate(noise::Config::new(keypair).unwrap())
                .multiplex(yamux)
                .timeout(std::time::Duration::from_secs(20))
                .boxed())
        }
    }

    fn new_swarm(
        config: Config,
        contacts: Arc<RwLock<PeerContactBook>>,
        peer_score_params: gossipsub::PeerScoreParams,
        force_dht_server_mode: bool,
    ) -> Swarm<behaviour::Behaviour> {
        let keypair = config.keypair.clone();
        let transport =
            Self::new_transport(&keypair, config.memory_transport, config.tls.as_ref()).unwrap();

        let behaviour =
            behaviour::Behaviour::new(config, contacts, peer_score_params, force_dht_server_mode);

        // TODO add proper config
        #[cfg(not(target_family = "wasm"))]
        let swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_other_transport(|_| transport)
            .unwrap()
            .with_behaviour(|_| behaviour)
            .unwrap()
            .build();
        #[cfg(target_family = "wasm")]
        let swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_wasm_bindgen()
            .with_other_transport(|_| transport)
            .unwrap()
            .with_behaviour(|_| behaviour)
            .unwrap()
            .build();
        swarm
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
        force_dht_server_mode: bool,
        dht_quorum: NonZeroU8,
        #[cfg(feature = "metrics")] metrics: Arc<NetworkMetrics>,
    ) {
        let mut task_state = TaskState {
            dht_server_mode: force_dht_server_mode,
            dht_quorum: dht_quorum.into(),
            ..Default::default()
        };

        let peer_id = Swarm::local_peer_id(&swarm);
        let task_span = trace_span!("swarm task", peer_id=?peer_id);

        async move {
            loop {
                tokio::select! {
                    validate_msg = validate_rx.recv() => {
                        if let Some(validate_msg) = validate_msg {
                            let topic = validate_msg.topic;
                            let result: Result<bool, gossipsub::PublishError> = swarm
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
        force_dht_server_mode: bool,
        dht_quorum: NonZeroU8,
        #[cfg(feature = "metrics")] metrics: Arc<NetworkMetrics>,
    ) {
        let mut task_state = TaskState {
            dht_server_mode: force_dht_server_mode,
            dht_quorum: dht_quorum.into(),
            ..Default::default()
        };

        let peer_id = Swarm::local_peer_id(&swarm);
        let task_span = trace_span!("swarm task", peer_id=?peer_id);

        async move {
            loop {
                tokio::select! {
                    validate_msg = validate_rx.recv() => {
                        if let Some(validate_msg) = validate_msg {
                            let topic = validate_msg.topic;
                            let result: Result<bool, gossipsub::PublishError> = swarm
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
        event: SwarmEvent<behaviour::BehaviourEvent>,
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
                connection_id,
                peer_id,
                endpoint,
                num_established,
                concurrent_dial_errors,
                established_in,
            } => {
                debug!(
                    %connection_id,
                    %peer_id,
                    address = %endpoint.get_remote_address(),
                    direction = if endpoint.is_dialer() { "outbound" } else { "inbound" },
                    connections = num_established,
                    ?established_in,
                    "Connection established",
                );

                if let Some(dial_errors) = concurrent_dial_errors {
                    for (addr, error) in dial_errors {
                        trace!(
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

                    if swarm.behaviour().is_address_dialable(listen_addr) {
                        debug!(%peer_id, address = %listen_addr, "Saving peer");

                        swarm
                            .behaviour_mut()
                            .add_peer_address(peer_id, listen_addr.clone());

                        // Bootstrap Kademlia if we're performing our first connection
                        if state.dht_bootstrap_state == DhtBootStrapState::NotStarted {
                            debug!("Bootstrapping DHT");
                            if swarm.behaviour_mut().dht.bootstrap().is_err() {
                                error!("Bootstrapping DHT error: No known peers");
                            }
                            state.dht_bootstrap_state = DhtBootStrapState::Started;
                        }
                    }
                }
            }

            SwarmEvent::ConnectionClosed {
                connection_id,
                peer_id,
                endpoint,
                num_established,
                cause,
            } => {
                info!(
                    %connection_id,
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

                    let _ = events_tx.send(NetworkEvent::PeerLeft(peer_id));
                }
            }
            SwarmEvent::IncomingConnection {
                connection_id,
                local_addr,
                send_back_addr,
            } => {
                debug!(
                    %connection_id,
                    address = %send_back_addr,
                    listen_address = %local_addr,
                    "Incoming connection",
                );
            }

            SwarmEvent::IncomingConnectionError {
                connection_id,
                local_addr,
                send_back_addr,
                error,
            } => {
                debug!(
                    %connection_id,
                    address = %send_back_addr,
                    listen_address = %local_addr,
                    %error,
                    "Incoming connection error",
                );
            }

            SwarmEvent::Dialing {
                peer_id,
                connection_id: _,
            } => {
                // This event is only triggered if the network behaviour performs the dial
                debug!(?peer_id, "Dialing peer");
            }

            SwarmEvent::NewListenAddr {
                listener_id: _,
                address,
            } => {
                debug!(%address, "New listen address");
                swarm
                    .behaviour_mut()
                    .discovery
                    .add_own_addresses([address].to_vec());
            }

            SwarmEvent::Behaviour(event) => {
                match event {
                    behaviour::BehaviourEvent::Autonat(event) => match event {
                        autonat::Event::InboundProbe(event) => {
                            log::trace!(?event, "Inbound probe");
                        }
                        autonat::Event::OutboundProbe(event) => {
                            log::trace!(?event, "Outbound probe");
                        }
                        autonat::Event::StatusChanged { old, new } => {
                            log::debug!(?old, ?new, "Autonat status changed");
                            if new == autonat::NatStatus::Private {
                                log::warn!("Couldn't detect a public reachable address. Validator network operations won't be possible");
                                log::warn!("You may need to find a relay to enable validator network operations");
                            }
                        }
                    },
                    behaviour::BehaviourEvent::ConnectionLimits(_) => {}
                    behaviour::BehaviourEvent::Dht(event) => {
                        match event {
                            kad::Event::OutboundQueryProgressed {
                                id,
                                result,
                                stats: _,
                                step,
                            } => {
                                match result {
                                    QueryResult::GetRecord(Ok(GetRecordOk::FoundRecord(
                                        record,
                                    ))) => {
                                        if let Some(dht_record) =
                                            Self::verify_record(&record.record)
                                        {
                                            if step.count.get() == 1_usize {
                                                // This is our first record
                                                let results = DhtResults {
                                                    count: 0, // Will be increased in the next step
                                                    best_value: dht_record.clone(),
                                                    outdated_values: vec![],
                                                };
                                                state.dht_get_results.insert(id, results);
                                            }
                                            // We should always have a stored result
                                            if let Some(results) =
                                                state.dht_get_results.get_mut(&id)
                                            {
                                                results.count += 1;
                                                // Replace best value if needed and update the outdated values
                                                if dht_record > results.best_value {
                                                    results
                                                        .outdated_values
                                                        .push(results.best_value.clone());
                                                    results.best_value = dht_record;
                                                } else if dht_record < results.best_value {
                                                    results.outdated_values.push(dht_record)
                                                }
                                                // Check if we already have a quorum
                                                if results.count == state.dht_quorum {
                                                    swarm
                                                        .behaviour_mut()
                                                        .dht
                                                        .query_mut(&id)
                                                        .unwrap()
                                                        .finish();
                                                    let signed_best_record = results
                                                        .best_value
                                                        .clone()
                                                        .get_signed_record();
                                                    // Send the best result to the application layer ASAP since we already know it
                                                    if let Some(output) = state.dht_gets.remove(&id)
                                                    {
                                                        if output
                                                            .send(Ok(signed_best_record
                                                                .clone()
                                                                .value))
                                                            .is_err()
                                                        {
                                                            error!(query_id = ?id, error = "receiver hung up", "could not send get record query result to channel");
                                                        }
                                                    } else {
                                                        warn!(query_id = ?id, ?step, "GetRecord query result for unknown query ID");
                                                    }
                                                    if !results.outdated_values.is_empty() {
                                                        // Now push the best value to the outdated peers
                                                        let outdated_peers = results
                                                            .outdated_values
                                                            .iter()
                                                            .map(|dht_record| {
                                                                dht_record.get_peer_id()
                                                            });
                                                        swarm.behaviour_mut().dht.put_record_to(
                                                            signed_best_record,
                                                            outdated_peers,
                                                            kad::Quorum::One,
                                                        );
                                                    }
                                                }
                                            } else {
                                                log::error!(query_id = ?id, "DHT inconsistent state");
                                            }
                                        } else {
                                            warn!(
                                                "DHT record verification failed: Invalid public key received"
                                            );
                                        }
                                    }
                                    QueryResult::GetRecord(Ok(
                                        GetRecordOk::FinishedWithNoAdditionalRecord {
                                            cache_candidates,
                                        },
                                    )) => {
                                        // Remove the query and push the best result to the cache candidates
                                        if let Some(results) = state.dht_get_results.remove(&id) {
                                            if !cache_candidates.is_empty() {
                                                let signed_best_record =
                                                    results.best_value.get_signed_record();
                                                let peers = cache_candidates
                                                    .iter()
                                                    .map(|(_, &peer_id)| peer_id);
                                                swarm.behaviour_mut().dht.put_record_to(
                                                    signed_best_record,
                                                    peers,
                                                    kad::Quorum::One,
                                                );
                                            }
                                        }
                                    }
                                    QueryResult::GetRecord(Err(error)) => {
                                        if let Some(output) = state.dht_gets.remove(&id) {
                                            if output.send(Err(error.clone().into())).is_err() {
                                                error!(query_id = ?id, query_error=?error, error = "receiver hung up", "could not send get record query result error to channel");
                                            }
                                        } else {
                                            warn!(query_id = ?id, ?step, query_error=?error, "GetRecord query result error for unknown query ID");
                                        }
                                        state.dht_get_results.remove(&id);
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
                                            if result.num_remaining == 0 {
                                                debug!(?result, "DHT bootstrap successful");
                                                state.dht_bootstrap_state =
                                                    DhtBootStrapState::Completed;
                                                if state.dht_server_mode {
                                                    let _ = events_tx.send(NetworkEvent::DhtReady);
                                                }
                                            }
                                        }
                                        Err(e) => error!(error = %e, "DHT bootstrap error"),
                                    },
                                    _ => {}
                                }
                            }
                            kad::Event::InboundRequest {
                                request:
                                    InboundRequest::PutRecord {
                                        source: _,
                                        connection: _,
                                        record: Some(record),
                                    },
                            } => {
                                // Verify incoming record
                                if let Some(dht_record) = Self::verify_record(&record) {
                                    // Now verify that we should overwrite it because it's better than the one we have
                                    let mut overwrite = true;
                                    let store = swarm.behaviour_mut().dht.store_mut();
                                    if let Some(current_record) = store.get(&record.key) {
                                        if let Ok(current_dht_record) =
                                            DhtRecord::try_from(&current_record.into_owned())
                                        {
                                            if current_dht_record > dht_record {
                                                overwrite = false;
                                            }
                                        }
                                    }
                                    if overwrite && store.put(record).is_err() {
                                        error!("Could not store record in DHT record store");
                                    }
                                } else {
                                    warn!(
                                        "DHT record verification failed: Invalid public key received"
                                    );
                                }
                            }
                            kad::Event::ModeChanged { new_mode } => {
                                debug!(%new_mode, "DHT mode changed");
                                if new_mode == kad::Mode::Server {
                                    state.dht_server_mode = true;
                                    if state.dht_bootstrap_state == DhtBootStrapState::Completed {
                                        let _ = events_tx.send(NetworkEvent::DhtReady);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    behaviour::BehaviourEvent::Discovery(event) => {
                        swarm.behaviour_mut().pool.maintain_peers();
                        match event {
                            Event::Established {
                                peer_id,
                                peer_address,
                                peer_contact,
                            } => {
                                let peer_info =
                                    PeerInfo::new(peer_address.clone(), peer_contact.services);
                                if connected_peers
                                    .write()
                                    .insert(peer_id, peer_info.clone())
                                    .is_none()
                                {
                                    info!(%peer_id, peer_address = %peer_info.get_address(), "Peer joined");
                                    let _ = events_tx
                                        .send(NetworkEvent::PeerJoined(peer_id, peer_info));

                                    if swarm.behaviour().is_address_dialable(&peer_address) {
                                        swarm
                                            .behaviour_mut()
                                            .add_peer_address(peer_id, peer_address);

                                        // Bootstrap Kademlia if we're adding our first address
                                        if state.dht_bootstrap_state
                                            == DhtBootStrapState::NotStarted
                                        {
                                            debug!("Bootstrapping DHT");
                                            if swarm.behaviour_mut().dht.bootstrap().is_err() {
                                                error!("Bootstrapping DHT error: No known peers");
                                            }
                                            state.dht_bootstrap_state = DhtBootStrapState::Started;
                                        }
                                    }
                                } else {
                                    error!(%peer_id, "Peer joined but it already exists");
                                }
                            }
                            Event::Update => {}
                        }
                    }
                    behaviour::BehaviourEvent::Gossipsub(event) => match event {
                        gossipsub::Event::Message {
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
                                            gossipsub::MessageAcceptance::Accept,
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
                        gossipsub::Event::Subscribed { peer_id, topic } => {
                            trace!(%peer_id, %topic, "peer subscribed to topic");
                        }
                        gossipsub::Event::Unsubscribed { peer_id, topic } => {
                            trace!(%peer_id, %topic, "peer unsubscribed");
                        }
                        gossipsub::Event::GossipsubNotSupported { peer_id } => {
                            debug!(%peer_id, "gossipsub not supported");
                        }
                    },
                    behaviour::BehaviourEvent::Ping(event) => {
                        match event.result {
                            Err(error) => {
                                log::debug!(%error, ?event.peer, "Ping failed with peer");
                                swarm
                                    .behaviour_mut()
                                    .pool
                                    .close_connection(event.peer, CloseReason::RemoteClosed);
                            }
                            Ok(duration) => {
                                log::trace!(?event.peer, ?duration, "Successful ping from peer");
                            }
                        };
                    }
                    behaviour::BehaviourEvent::Pool(event) => {
                        match event {
                            connection_pool::Event::PeerJoined { peer_id: _ } => {}
                        };
                    }
                    behaviour::BehaviourEvent::RequestResponse(event) => match event {
                        request_response::Event::Message {
                            peer: peer_id,
                            message,
                        } => match message {
                            request_response::Message::Request {
                                request_id,
                                request,
                                channel,
                            } => {
                                // We might get empty requests (None) because of our codec implementation
                                if let Some(request) = request {
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
                                        let sender = match state.receive_requests.get_mut(&type_id)
                                        {
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
                                                let response: Result<(), InboundRequestError> =
                                                    Ok(());
                                                if swarm
                                                    .behaviour_mut()
                                                    .request_response
                                                    .send_response(
                                                        channel,
                                                        Some(response.serialize_to_vec()),
                                                    )
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
                                            if let Err(e) = sender.try_send((
                                                request.into(),
                                                request_id,
                                                peer_id,
                                            )) {
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
                                                .send_response(
                                                    channel,
                                                    Some(err.serialize_to_vec()),
                                                )
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
                            }
                            request_response::Message::Response {
                                request_id,
                                response,
                            } => {
                                trace!(
                                    %request_id,
                                    %peer_id,
                                    "Incoming response from peer",
                                );
                                if let Some(channel) = state.requests.remove(&request_id) {
                                    // We might get empty responses (None) because of the implementation of our codecs.
                                    if channel
                                        .send(
                                            response
                                                .ok_or(RequestError::OutboundRequest(
                                                    OutboundRequestError::Timeout,
                                                ))
                                                .map(|data| data.into()),
                                        )
                                        .is_err()
                                    {
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
                        request_response::Event::OutboundFailure {
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
                        request_response::Event::InboundFailure {
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
                        request_response::Event::ResponseSent { peer, request_id } => {
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
                        Swarm::dial(
                            swarm,
                            DialOpts::peer_id(peer_id)
                                .condition(PeerCondition::Disconnected)
                                .build(),
                        )
                        .map_err(Into::into),
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
                let topic = gossipsub::IdentTopic::new(topic_name.clone());

                match swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                    // New subscription. Insert the sender into our subscription table.
                    Ok(true) => {
                        let (tx, rx) = mpsc::channel(buffer_size);

                        state.gossip_topics.insert(topic.hash(), (tx, validate));

                        match swarm
                            .behaviour_mut()
                            .gossipsub
                            .set_topic_params(topic, gossipsub::TopicScoreParams::default())
                        {
                            Ok(_) => {
                                if output.send(Ok(rx)).is_err() {
                                    error!(%topic_name, error = "receiver hung up", "could not send subscribe result to channel");
                                }
                            }
                            Err(e) => {
                                if output
                                    .send(Err(NetworkError::TopicScoreParams {
                                        topic_name: topic_name.clone(),
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
                            .send(Err(NetworkError::AlreadySubscribed {
                                topic_name: topic_name.clone(),
                            }))
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
                let topic = gossipsub::IdentTopic::new(topic_name.clone());

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
                                .send(Err(NetworkError::AlreadyUnsubscribed {
                                    topic_name: topic_name.clone(),
                                }))
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
                        .send(Err(NetworkError::AlreadyUnsubscribed {
                            topic_name: topic_name.clone(),
                        }))
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
                let topic = gossipsub::IdentTopic::new(topic_name.clone());

                if output
                    .send(
                        swarm
                            .behaviour_mut()
                            .gossipsub
                            .publish(topic, data)
                            .map(|_| ())
                            .or_else(|e| match e {
                                gossipsub::PublishError::Duplicate => Ok(()),
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
                    .send_request(&peer_id, Some(request));
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
                                .send_response(response_channel, Some(response))
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
            NetworkAction::ConnectPeersByServices {
                services,
                num_peers,
                output,
            } => {
                let peers_candidates = swarm
                    .behaviour_mut()
                    .pool
                    .choose_peers_to_dial_by_services(services, num_peers);
                let mut successful_peers = vec![];

                for peer_id in peers_candidates {
                    if Swarm::dial(
                        swarm,
                        DialOpts::peer_id(peer_id)
                            .condition(PeerCondition::Disconnected)
                            .build(),
                    )
                    .is_ok()
                    {
                        successful_peers.push(peer_id);
                    }
                }

                if output.send(successful_peers).is_err() {
                    error!("Could not send sucessful peers vector");
                }
            }
            NetworkAction::DisconnectPeer { peer_id, reason } => {
                swarm.behaviour_mut().pool.close_connection(peer_id, reason)
            }
            NetworkAction::UnbanPeer { peer_id } => {
                swarm.behaviour_mut().pool.unban_connection(peer_id)
            }
        }
    }

    /// Gets the network information
    pub async fn network_info(&self) -> Result<NetworkInfo, NetworkError> {
        let (output_tx, output_rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::NetworkInfo { output: output_tx })
            .await?;
        Ok(output_rx.await?)
    }

    /// Tells the network to listen on a specific address received in a
    /// `Multiaddr` format.
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

    /// Tells the network to start connecting to any available peer or seed
    /// until meeting the configured number of desired peer connections.
    /// If there are no dial attempts being made and no connections to any
    /// peer it clears the internal set of peers or addresses marked as down.
    /// This is useful to instruct the network to reconnect after issuing a
    /// `stop_connecting` call.
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

    /// Tells the network to un-ban a peer ID
    pub async fn unban_peer(&self, peer_id: PeerId) {
        if let Err(error) = self
            .action_tx
            .clone()
            .send(NetworkAction::UnbanPeer { peer_id })
            .await
        {
            error!(%error, "Failed to send NetworkAction::UnbanPeer");
        }
    }

    async fn request_impl<Req: RequestCommon>(
        &self,
        request: Req,
        peer_id: PeerId,
    ) -> Result<Req::Response, RequestError> {
        let (output_tx, output_rx) = oneshot::channel();
        let (response_tx, response_rx) = oneshot::channel();

        let buf = request.serialize_request();

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
                    if let Ok((message, left_over)) =
                        <Result<Req::Response, InboundRequestError>>::deserialize_take(&data)
                    {
                        if !left_over.is_empty() {
                            warn!(
                            %request_id,
                            %peer_id,
                            type_id = std::any::type_name::<Req::Response>(),
                            unread_data_len = left_over.len(),
                            "Unexpected content size deserializing",
                            );
                        }
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
    ) -> BoxStream<'static, (Req, InboundRequestId, PeerId)> {
        enum ReceiveStream {
            WaitingForRegister(
                BoxFuture<'static, mpsc::Receiver<(Bytes, InboundRequestId, PeerId)>>,
            ),
            Registered(mpsc::Receiver<(Bytes, InboundRequestId, PeerId)>),
        }

        impl Stream for ReceiveStream {
            type Item = (Bytes, InboundRequestId, PeerId);
            fn poll_next(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<Option<(Bytes, InboundRequestId, PeerId)>> {
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
                match Req::deserialize_request(&data) {
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
            OutboundFailure::Io(error) => {
                RequestError::OutboundRequest(OutboundRequestError::Other(error.to_string()))
            }
        }
    }

    /// Returns a DHT record if the record decoding and verification was successful, None otherwise
    fn verify_record(record: &Record) -> Option<DhtRecord> {
        if let Some(tag) = TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::peek_tag(&record.value)
        {
            match tag {
                ValidatorRecord::<PeerId>::TAG => {
                    if let Ok(validator_record) =
                        TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::deserialize_from_vec(
                            &record.value,
                        )
                    {
                        // In this type of messages we assume the record key is also the public key used to verify these records
                        if let Ok(compressed_pk) =
                            CompressedPublicKey::deserialize_from_vec(record.key.as_ref())
                        {
                            if let Ok(pk) = compressed_pk.uncompress() {
                                if validator_record.verify(&pk) {
                                    return Some(DhtRecord::Validator(
                                        record.publisher.unwrap(),
                                        validator_record.record,
                                        record.clone(),
                                    ));
                                }
                            }
                        }
                    }
                }
                _ => {
                    log::error!(tag, "DHT invalid record tag received");
                }
            }
        }

        // If we arrived here, it's because something failed in the record verification
        None
    }

    /// Gets the number of connected peers
    pub fn peer_count(&self) -> usize {
        self.connected_peers.read().len()
    }

    /// Disconnects from (closes the connection to) all peers with a reason
    pub async fn disconnect(&self, reason: CloseReason) {
        for peer_id in self.get_peers() {
            self.disconnect_peer(peer_id, reason).await;
        }
    }

    #[cfg(feature = "metrics")]
    /// Gets the network metrics
    pub fn metrics(&self) -> Arc<NetworkMetrics> {
        self.metrics.clone()
    }

    fn is_under_the_rate_limits<Req: RequestCommon>(
        peer_request_limits: Arc<Mutex<HashMap<PeerId, HashMap<u16, RateLimit>>>>,
        peer_id: PeerId,
        request_id: InboundRequestId,
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
        request_id: InboundRequestId,
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

    async fn subscribe_with_name<T>(
        &self,
        topic_name: String,
    ) -> Result<BoxStream<'static, (T::Item, GossipsubId<PeerId>)>, NetworkError>
    where
        T: Topic + Sync,
    {
        let (tx, rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::Subscribe {
                topic_name,
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

    async fn unsubscribe_with_name(&self, topic_name: String) -> Result<(), NetworkError> {
        let (output_tx, output_rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::Unsubscribe {
                topic_name,
                output: output_tx,
            })
            .await?;

        output_rx.await?
    }

    async fn publish_with_name<T>(
        &self,
        topic_name: String,
        item: <T as Topic>::Item,
    ) -> Result<(), NetworkError>
    where
        T: Topic + Sync,
    {
        let (output_tx, output_rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::Publish {
                topic_name,
                data: item.serialize_to_vec(),
                output: output_tx,
            })
            .await?;

        output_rx.await??;

        #[cfg(feature = "metrics")]
        self.metrics
            .note_published_pubsub_message(<T as Topic>::NAME);

        Ok(())
    }
}

#[async_trait]
impl NetworkInterface for Network {
    type PeerId = PeerId;
    type AddressType = Multiaddr;
    type Error = NetworkError;
    type PubsubId = GossipsubId<PeerId>;
    type RequestId = InboundRequestId;

    fn get_peers(&self) -> Vec<PeerId> {
        self.connected_peers.read().keys().copied().collect()
    }

    fn has_peer(&self, peer_id: PeerId) -> bool {
        self.connected_peers.read().contains_key(&peer_id)
    }

    fn get_peer_info(&self, peer_id: Self::PeerId) -> Option<PeerInfo> {
        self.connected_peers.read().get(&peer_id).cloned()
    }

    async fn get_peers_by_services(
        &self,
        services: Services,
        min_peers: usize,
    ) -> Result<Vec<Self::PeerId>, NetworkError> {
        let (output_tx, output_rx) = oneshot::channel();
        let connected_peers = self.get_peers();
        let mut filtered_peers = vec![];

        // First we try to get the connected peers that support the desired services
        for peer_id in connected_peers.iter() {
            if let Some(peer_info) = self.get_peer_info(*peer_id) {
                if peer_info.get_services().contains(services) {
                    filtered_peers.push(*peer_id);
                }
            }
        }

        // If we don't have enough connected peers that support the desired services,
        // we tell the network to connect to new peers that support such services.
        if filtered_peers.len() < min_peers {
            let num_peers = min_peers - filtered_peers.len();

            self.action_tx
                .send(NetworkAction::ConnectPeersByServices {
                    services,
                    num_peers,
                    output: output_tx,
                })
                .await?;

            filtered_peers.extend(output_rx.await?.iter());
        }

        // If filtered_peers is still less than the minimum required,
        // we return an error.
        if filtered_peers.len() < min_peers {
            return Err(NetworkError::PeersNotFound);
        }

        Ok(filtered_peers)
    }

    fn peer_provides_required_services(&self, peer_id: PeerId) -> bool {
        if let Some(peer_info) = self.connected_peers.read().get(&peer_id) {
            peer_info.get_services().contains(self.required_services)
        } else {
            // If we don't know the peer we return false
            false
        }
    }

    fn peer_provides_services(&self, peer_id: PeerId, services: Services) -> bool {
        if let Some(peer_info) = self.connected_peers.read().get(&peer_id) {
            peer_info.get_services().contains(services)
        } else {
            // If we don't know the peer we return false
            false
        }
    }

    async fn disconnect_peer(&self, peer_id: PeerId, close_reason: CloseReason) {
        if let Err(error) = self
            .action_tx
            .send(NetworkAction::DisconnectPeer {
                peer_id,
                reason: close_reason,
            })
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
        let topic_name = <T as Topic>::NAME.to_string();

        self.subscribe_with_name::<T>(topic_name).await
    }

    async fn unsubscribe<T>(&self) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = <T as Topic>::NAME.to_string();
        self.unsubscribe_with_name(topic_name).await
    }

    async fn publish<T>(&self, item: <T as Topic>::Item) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = <T as Topic>::NAME.to_string();
        self.publish_with_name::<T>(topic_name, item).await
    }

    async fn subscribe_subtopic<T>(
        &self,
        subtopic: String,
    ) -> Result<BoxStream<'static, (T::Item, Self::PubsubId)>, Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = format!("{}_{}", <T as Topic>::NAME, subtopic);

        self.subscribe_with_name::<T>(topic_name).await
    }

    async fn unsubscribe_subtopic<T>(&self, subtopic: String) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = format!("{}_{}", <T as Topic>::NAME, subtopic);
        self.unsubscribe_with_name(topic_name).await
    }

    async fn publish_subtopic<T>(
        &self,
        subtopic: String,
        item: <T as Topic>::Item,
    ) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = format!("{}_{}", <T as Topic>::NAME, subtopic);
        self.publish_with_name::<T>(topic_name, item).await
    }

    fn validate_message<T>(&self, pubsub_id: Self::PubsubId, acceptance: MsgAcceptance)
    where
        T: Topic + Sync,
    {
        self.validate_tx
            .send(ValidateMessage::new::<T>(pubsub_id, acceptance))
            .expect("Failed to send reported message validation result: receiver hung up");
    }

    async fn dht_get<K, V, T>(&self, k: &K) -> Result<Option<V>, Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Deserialize + Send + Sync + TaggedSignable + Ord,
        T: TaggedKeyPair + Send + Sync + Serialize + Deserialize,
    {
        let (output_tx, output_rx) = oneshot::channel();
        self.action_tx
            .clone()
            .send(NetworkAction::DhtGet {
                key: k.as_ref().to_owned(),
                output: output_tx,
            })
            .await?;

        let data = output_rx.await??;
        // Now decode the signed record and returned the tagged signable record
        let signed_record: TaggedSigned<V, T> = Deserialize::deserialize_from_vec(&data)?;
        Ok(Some(signed_record.record))
    }

    async fn dht_put<K, V, T>(&self, k: &K, v: &V, keypair: &T) -> Result<(), Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Serialize + Send + Sync + TaggedSignable + Clone + Ord,
        T: TaggedKeyPair + Send + Sync + Serialize + Deserialize,
    {
        // Sign the record before transmitting it to the swarm
        let signature = keypair.tagged_sign(v);
        let signed_record = TaggedSigned::new(v.clone(), signature);
        let (output_tx, output_rx) = oneshot::channel();

        self.action_tx
            .clone()
            .send(NetworkAction::DhtPut {
                key: k.as_ref().to_owned(),
                value: signed_record.serialize_to_vec(),
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

    fn receive_requests<Req: Request>(
        &self,
    ) -> BoxStream<'static, (Req, InboundRequestId, PeerId)> {
        self.receive_requests_impl()
    }

    async fn respond<Req: Request>(
        &self,
        request_id: InboundRequestId,
        response: Req::Response,
    ) -> Result<(), Self::Error> {
        let (output_tx, output_rx) = oneshot::channel();

        // Encapsulate it in a `Result` to signal the network that this
        // was a successful response from the application.
        let response: Result<Req::Response, InboundRequestError> = Ok(response);
        let ser_response = response.serialize_to_vec();

        self.action_tx
            .clone()
            .send(NetworkAction::SendResponse {
                request_id,
                response: ser_response,
                output: output_tx,
            })
            .await?;

        output_rx.await?
    }
}
