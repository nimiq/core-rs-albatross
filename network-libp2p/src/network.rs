use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{future::BoxFuture, ready, stream::BoxStream, Stream, StreamExt};
#[cfg(all(target_family = "wasm", not(feature = "tokio-websocket")))]
use libp2p::websocket_websys;
use libp2p::{
    gossipsub, request_response::InboundRequestId, swarm::NetworkInfo, Multiaddr, PeerId, Swarm,
};
use nimiq_network_interface::{
    network::{
        CloseReason, MsgAcceptance, Network as NetworkInterface, NetworkEvent, SubscribeEvents,
        Topic,
    },
    peer_info::{PeerInfo, Services},
    request::{
        InboundRequestError, Message, OutboundRequestError, Request, RequestCommon, RequestError,
        RequestSerialize, RequestType,
    },
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_time::{interval, timeout};
use nimiq_utils::{
    spawn::spawn,
    tagged_signing::{TaggedKeyPair, TaggedSignable, TaggedSigned},
};
use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};

#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::{
    discovery::peer_contacts::PeerContactBook,
    network_types::{GossipsubId, NetworkAction, ValidateMessage},
    rate_limiting::RequestRateLimitData,
    swarm::{new_swarm, swarm_task},
    Config, NetworkError,
};

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
    /// Metrics used for data analysis
    #[cfg(feature = "metrics")]
    metrics: Arc<NetworkMetrics>,
    /// Required services from other peers. This is defined on init, based on our client type
    required_services: Services,
    /// Reference to PeerContactBook, used to satisfy rpc requests for it.
    contacts: Arc<RwLock<PeerContactBook>>,
}

impl Network {
    /// Create a new libp2p network instance.
    ///
    /// # Arguments
    ///
    ///  - `config`: The network configuration, containing key pair, and other behavior-specific configuration.
    ///
    pub async fn new(config: Config) -> Self {
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
        let swarm = new_swarm(
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

        let update_scores = interval(params.decay_interval);

        #[cfg(feature = "metrics")]
        let metrics = Arc::new(NetworkMetrics::default());

        spawn(Box::pin(swarm_task(
            swarm,
            events_tx.clone(),
            action_rx,
            validate_rx,
            Arc::clone(&connected_peers),
            update_scores,
            Arc::clone(&contacts),
            force_dht_server_mode,
            dht_quorum,
            #[cfg(feature = "metrics")]
            metrics.clone(),
        )));

        Self {
            contacts,
            local_peer_id,
            connected_peers,
            events_tx,
            action_tx,
            validate_tx,
            #[cfg(feature = "metrics")]
            metrics,
            required_services,
        }
    }

    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    /// Retrieves a single PeerInfo peer existing in the PeerAddressBook.
    /// If that peer has multiple associated addresses all but the first are omitted.
    pub fn get_address_book(&self) -> Vec<(PeerId, PeerInfo)> {
        self.contacts.read().known_peers()
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
            let result = match timeout(Req::TIME_WINDOW.mul_f32(1.5f32), response_rx).await {
                Ok(res) => res,
                Err(_) => {
                    warn!(
                        %request_id,
                        request_type = std::any::type_name::<Req>(),
                        %peer_id,
                        "Request timed out with no response from libp2p"
                    );
                    return Err(RequestError::OutboundRequest(
                        OutboundRequestError::SenderFutureDropped,
                    ));
                }
            };

            match result {
                Err(_) => {
                    debug!(
                        %request_id,
                        request_type = std::any::type_name::<Req>(),
                        %peer_id,
                        "Request failed - sender future dropped"
                    );
                    Err(RequestError::OutboundRequest(
                        OutboundRequestError::SenderFutureDropped,
                    ))
                }
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

        let action_tx = self.action_tx.clone();
        ReceiveStream::WaitingForRegister(Box::pin(async move {
            // TODO Make buffer size configurable
            let (tx, rx) = mpsc::channel(1024);

            action_tx
                .send(NetworkAction::ReceiveRequests {
                    type_id: RequestType::from_request::<Req>(),
                    output: tx,
                    request_rate_limit_data: RequestRateLimitData::new::<Req>(),
                })
                .await
                .expect("Sending action to network task failed.");

            rx
        }))
        .filter_map(move |(data, request_id, peer_id)| {
            async move {
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

        Ok(Box::pin(subscribe_rx.filter_map(
            |(msg, msg_id, source)| async move {
                let item: <T as Topic>::Item = Deserialize::deserialize_from_vec(&msg.data).ok()?;
                let id = GossipsubId {
                    message_id: msg_id,
                    propagation_source: source,
                };
                Some((item, id))
            },
        )))
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
