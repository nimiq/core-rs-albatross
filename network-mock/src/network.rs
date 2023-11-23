use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use nimiq_network_interface::{
    network::{
        CloseReason, MsgAcceptance, Network, NetworkEvent, PubsubId, SubscribeEvents, Topic,
    },
    peer_info::{PeerInfo, Services},
    request::{
        InboundRequestError, Message, OutboundRequestError, Request, RequestCommon, RequestError,
        RequestKind, RequestSerialize, RequestType,
    },
};
use nimiq_serde::{Deserialize, DeserializeError, Serialize};
use parking_lot::{Mutex, RwLock};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream, ReceiverStream};

use crate::{
    hub::{MockHubInner, RequestKey, ResponseSender},
    observable_hash_map, MockAddress, MockPeerId, ObservableHashMap,
};

#[derive(Debug, Error, Eq, PartialEq)]
pub enum MockNetworkError {
    #[error("Serialization error: {0}")]
    Serialization(#[from] DeserializeError),

    #[error("Can't connect to peer: {0}")]
    CantConnect(MockAddress),

    #[error("Network is not connected")]
    NotConnected,

    #[error("Peer is already subscribed to topic: {0}")]
    AlreadySubscribed(String),

    #[error("Peer is already unsubscribed to topic: {0}")]
    AlreadyUnsubscribed(String),

    #[error("Can't respond to request: {0}")]
    CantRespond(MockRequestId),
}

pub type MockRequestId = u64;

#[derive(Clone, Debug)]
pub struct MockId<P> {
    propagation_source: P,
}

impl MockId<MockPeerId> {
    pub fn new(propagation_source: MockPeerId) -> Self {
        Self { propagation_source }
    }
}

impl PubsubId<MockPeerId> for MockId<MockPeerId> {
    fn propagation_source(&self) -> MockPeerId {
        self.propagation_source
    }
}

#[derive(Debug)]
pub struct MockNetwork {
    address: MockAddress,
    peers: Arc<RwLock<ObservableHashMap<MockPeerId, PeerInfo>>>,
    hub: Arc<Mutex<MockHubInner>>,
    is_connected: Arc<AtomicBool>,
}

impl MockNetwork {
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

    pub(crate) fn new(address: MockAddress, hub: Arc<Mutex<MockHubInner>>) -> Self {
        let peers = Arc::new(RwLock::new(ObservableHashMap::new()));

        let is_connected = {
            let mut hub = hub.lock();

            // Insert out peer map into global peer maps table
            if hub.peer_maps.insert(address, peers.clone()).is_some() {
                panic!("address/peer_id of MockNetwork must be unique: address={address}");
            }

            // Insert our is_connected bool into the hub
            let is_connected = Arc::new(AtomicBool::new(false));
            hub.is_connected.insert(address, Arc::clone(&is_connected));

            is_connected
        };

        Self {
            address,
            peers,
            hub,
            is_connected,
        }
    }

    pub fn address(&self) -> MockAddress {
        self.address
    }

    pub fn peer_id(&self) -> MockPeerId {
        self.address.into()
    }

    fn dial_mock_address(&self, address: MockAddress) -> Result<(), MockNetworkError> {
        let hub = self.hub.lock();

        log::debug!("Peer {} dialing peer {}", self.address, address);

        // Insert ourselves into peer's peer list.
        // This also makes sure the other peer actually exists.
        let is_new;
        {
            let mut other_peers = hub
                .peer_maps
                .get(&address)
                .ok_or(MockNetworkError::CantConnect(address))?
                .write();
            is_new = !other_peers.contains_key(&self.peer_id());
            if is_new {
                let peer_info = PeerInfo::new(address.into(), Services::all());
                other_peers.insert(self.peer_id(), peer_info);
            }
        }

        if is_new {
            // Set is_connected flag for this network
            self.is_connected.store(true, Ordering::SeqCst);

            // Are we connecting to someone that is not ourselves?
            if self.address != address {
                // Insert peer into our peer list
                let peer_info = PeerInfo::new(address.into(), Services::all());
                assert!(self
                    .peers
                    .write()
                    .insert(address.into(), peer_info)
                    .is_none());

                // Set is_connected flag for other network
                hub.is_connected
                    .get(&address)
                    .unwrap()
                    .store(true, Ordering::SeqCst);
            }
        } else {
            log::trace!("Peers are already connected.");
        }

        Ok(())
    }

    /// Dials another mock network. Might panic if the peers are not in the same hub (i.e. if the address of the
    /// other network doesn't exist in our hub).
    pub fn dial_mock(&self, other: &Self) {
        self.dial_mock_address(other.address).unwrap();
    }

    async fn request_impl<Req: RequestCommon>(
        &self,
        request: Req,
        peer_id: MockPeerId,
    ) -> Result<Req::Response, RequestError> {
        if !self.peers.read().contains_key(&peer_id) {
            log::warn!(
                "Cannot send request {} from {} to {} - peers not connected",
                std::any::type_name::<Req>(),
                self.address,
                peer_id,
            );
            return Err(RequestError::OutboundRequest(
                OutboundRequestError::SendError,
            ));
        }

        let sender_id = MockPeerId::from(self.address);
        let (tx, rx) = oneshot::channel::<Vec<u8>>();

        let (sender, request_id) = {
            let mut hub = self.hub.lock();

            let key = RequestKey {
                recipient: peer_id.into(),
                message_type: RequestType::from_request::<Req>(),
            };
            let sender = if let Some(sender) = hub.request_senders.get(&key) {
                sender.clone()
            } else {
                log::warn!("No request sender: {:?}", key);
                return Err(RequestError::OutboundRequest(
                    OutboundRequestError::SendError,
                ));
            };

            let request_id = hub.next_request_id;
            if Req::Kind::EXPECT_RESPONSE {
                hub.response_senders.insert(
                    request_id,
                    ResponseSender {
                        peer: self.address.into(),
                        sender: tx,
                    },
                );
            } else {
                let response: Result<(), InboundRequestError> = Ok(());
                tx.send(response.serialize_to_vec()).unwrap();
            }
            hub.next_request_id += 1;

            (sender, request_id)
        };

        let data = request.serialize_request();

        let request = (data, request_id, sender_id);
        if let Err(e) = sender.send(request).await {
            log::warn!(
                "Cannot send request {} from {} to {} - {:?}",
                std::any::type_name::<Req>(),
                self.address,
                peer_id,
                e
            );
            self.hub.lock().response_senders.remove(&request_id);
            return Err(RequestError::OutboundRequest(
                OutboundRequestError::SendError,
            ));
        }

        let result = tokio::time::timeout(MockNetwork::REQUEST_TIMEOUT, rx).await;
        match result {
            Ok(Ok(data)) => match Req::Response::deserialize_from_vec(&data[..]) {
                Ok(message) => Ok(message),
                Err(_) => Err(RequestError::InboundRequest(
                    InboundRequestError::DeSerializationError,
                )),
            },
            Ok(Err(_)) => Err(RequestError::InboundRequest(
                InboundRequestError::SenderFutureDropped,
            )),
            Err(_) => {
                self.hub.lock().response_senders.remove(&request_id);
                Err(RequestError::InboundRequest(InboundRequestError::Timeout))
            }
        }
    }

    fn receive_requests_impl<Req: RequestCommon>(
        &self,
    ) -> BoxStream<'static, (Req, MockRequestId, MockPeerId)> {
        let mut hub = self.hub.lock();
        let (tx, rx) = mpsc::channel(16);

        let key = RequestKey {
            recipient: self.address,
            message_type: RequestType::from_request::<Req>(),
        };
        if hub.request_senders.insert(key, tx).is_some() {
            log::warn!(
                "Replacing existing request sender for {}",
                std::any::type_name::<Req>()
            );
        }

        ReceiverStream::new(rx)
            .filter_map(|(data, request_id, sender)| async move {
                match Req::deserialize_request(&data) {
                    Ok(message) => Some((message, request_id, sender)),
                    Err(e) => {
                        log::warn!("Failed to deserialize request: {}", e);
                        None
                    }
                }
            })
            .boxed()
    }

    /// Disconnect from all peers
    pub fn disconnect(&self) {
        let hub = self.hub.lock();

        let mut peers = self.peers.write();
        let peer_ids: Vec<_> = peers.keys().copied().collect();
        for peer_id in peer_ids {
            peers.remove(&peer_id);
            let mut peer_map = hub.peer_maps.get(&peer_id.into()).unwrap_or_else(|| {
                panic!(
                    "We're connected to a peer that doesn't have a connection to us: our_peer_id={}, their_peer_id={}",
                    self.address,
                    peer_id,
                );
            }).write();
            peer_map.remove(&self.address.into());
        }

        self.is_connected.store(false, Ordering::SeqCst);
    }

    /// Disconnects from all peers and deletes this peer from the hub to prevent future connections
    /// to or from it.
    pub fn shutdown(&self) {
        self.disconnect();
        self.hub.lock().peer_maps.remove(&self.address);
    }

    async fn subscribe_with_name<T>(
        &self,
        topic_name: String,
    ) -> Result<BoxStream<'static, (T::Item, MockId<MockPeerId>)>, MockNetworkError>
    where
        T: Topic + Sync,
    {
        let mut hub = self.hub.lock();
        let is_connected = Arc::clone(&self.is_connected);

        log::debug!(
            "Peer {} subscribing to topic '{}'",
            self.address,
            topic_name
        );

        // Add this peer to the topic list
        let sender: &broadcast::Sender<(Arc<Vec<u8>>, MockPeerId)> =
            if let Some(topic) = hub.subscribe(topic_name.clone(), self.address) {
                &topic.sender
            } else {
                return Err(MockNetworkError::AlreadySubscribed(topic_name));
            };

        let stream = BroadcastStream::new(sender.subscribe()).filter_map(move |r| {
            let is_connected = Arc::clone(&is_connected);

            async move {
                if is_connected.load(Ordering::SeqCst) {
                    match r {
                        Ok((data, peer_id)) => match T::Item::deserialize_from_vec(&data) {
                            Ok(item) => return Some((item, peer_id)),
                            Err(e) => {
                                log::warn!("Dropped item because deserialization failed: {}", e)
                            }
                        },
                        Err(BroadcastStreamRecvError::Lagged(_)) => {
                            log::warn!("Mock gossipsub channel is lagging")
                        }
                    }
                } else {
                    log::debug!("Network not connected: Dropping gossipsub message.");
                }

                None
            }
        });

        Ok(Box::pin(stream.map(|(topic, peer_id)| {
            let id = MockId {
                propagation_source: peer_id,
            };
            (topic, id)
        })))
    }

    async fn unsubscribe_with_name(&self, topic_name: String) -> Result<(), MockNetworkError> {
        let mut hub = self.hub.lock();

        log::debug!(
            "Peer {} unsubscribing from topic '{}'",
            self.address,
            topic_name
        );

        if self.is_connected.load(Ordering::SeqCst) {
            if hub.unsubscribe(&topic_name, &self.address) {
                Ok(())
            } else {
                Err(MockNetworkError::AlreadyUnsubscribed(topic_name))
            }
        } else {
            Err(MockNetworkError::NotConnected)
        }
    }

    async fn publish_with_name<T: Topic>(
        &self,
        topic_name: String,
        item: T::Item,
    ) -> Result<(), MockNetworkError>
    where
        T: Topic + Sync,
    {
        let mut hub = self.hub.lock();

        let data = item.serialize_to_vec();

        log::debug!(
            "Peer {} publishing on topic '{}': {:?}",
            self.address,
            topic_name,
            item
        );

        if self.is_connected.load(Ordering::SeqCst) {
            if let Some(topic) = hub.get_topic(&topic_name) {
                topic
                    .sender
                    .send((Arc::new(data), self.address.into()))
                    .unwrap();
                Ok(())
            } else {
                log::debug!("No peer is subscribed to topic: '{}'", topic_name);
                Ok(())
            }
        } else {
            Err(MockNetworkError::NotConnected)
        }
    }
}

#[async_trait]
impl Network for MockNetwork {
    type PeerId = MockPeerId;
    type AddressType = MockAddress;
    type Error = MockNetworkError;
    type PubsubId = MockId<MockPeerId>;
    type RequestId = MockRequestId;

    fn get_peers(&self) -> Vec<MockPeerId> {
        self.peers.read().keys().copied().collect()
    }

    fn has_peer(&self, peer_id: MockPeerId) -> bool {
        self.peers.read().get(&peer_id).is_some()
    }

    async fn disconnect_peer(&self, peer_id: MockPeerId, _: CloseReason) {
        if !self.has_peer(peer_id) {
            return;
        }

        let mut hub = self.hub.lock();

        // Drops senders and thus the receiver stream will end
        hub.network_senders
            .retain(|k, _| k.network_recipient != peer_id.into());
    }

    fn subscribe_events(&self) -> SubscribeEvents<MockPeerId> {
        Box::pin(
            BroadcastStream::new(self.peers.read().subscribe()).map(|maybe_ev| {
                maybe_ev.map(|ev| match ev {
                    observable_hash_map::Event::Add(peer_id, peer_info) => {
                        NetworkEvent::PeerJoined(peer_id, peer_info)
                    }
                    observable_hash_map::Event::Remove(peer_id) => NetworkEvent::PeerLeft(peer_id),
                })
            }),
        )
    }

    async fn subscribe<T>(
        &self,
    ) -> Result<BoxStream<'static, (T::Item, Self::PubsubId)>, Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = T::NAME.to_string();
        self.subscribe_with_name::<T>(topic_name).await
    }

    async fn subscribe_subtopic<T>(
        &self,
        subtopic: String,
    ) -> Result<BoxStream<'static, (T::Item, Self::PubsubId)>, Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = format!("{}_{}", T::NAME, subtopic);
        self.subscribe_with_name::<T>(topic_name).await
    }

    async fn unsubscribe<T>(&self) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = T::NAME.to_string();
        self.unsubscribe_with_name(topic_name).await
    }

    async fn unsubscribe_subtopic<T>(&self, subtopic: String) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = format!("{}_{}", T::NAME, subtopic);
        self.unsubscribe_with_name(topic_name).await
    }

    async fn publish<T: Topic>(&self, item: T::Item) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = T::NAME.to_string();
        self.publish_with_name::<T>(topic_name, item).await
    }

    async fn publish_subtopic<T: Topic>(
        &self,
        subtopic: String,
        item: T::Item,
    ) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let topic_name = format!("{}_{}", T::NAME, subtopic);
        self.publish_with_name::<T>(topic_name, item).await
    }

    fn validate_message<TTopic>(&self, _id: Self::PubsubId, _acceptance: MsgAcceptance)
    where
        TTopic: Topic + Sync,
    {
        // TODO implement
    }

    async fn dht_get<K, V>(&self, k: &K) -> Result<Option<V>, Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Deserialize + Send + Sync,
    {
        if self.is_connected.load(Ordering::SeqCst) {
            let hub = self.hub.lock();

            if let Some(data) = hub.dht.get(k.as_ref()) {
                Ok(Some(V::deserialize_from_vec(data)?))
            } else {
                Ok(None)
            }
        } else {
            Err(MockNetworkError::NotConnected)
        }
    }

    async fn dht_put<K, V>(&self, k: &K, v: &V) -> Result<(), Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Serialize + Send + Sync,
    {
        if self.is_connected.load(Ordering::SeqCst) {
            let mut hub = self.hub.lock();

            let data = v.serialize_to_vec();
            hub.dht.insert(k.as_ref().to_owned(), data);
            Ok(())
        } else {
            Err(MockNetworkError::NotConnected)
        }
    }

    async fn dial_peer(&self, peer_id: MockPeerId) -> Result<(), Self::Error> {
        self.dial_mock_address(peer_id.into())
    }

    async fn dial_address(&self, address: MockAddress) -> Result<(), Self::Error> {
        self.dial_mock_address(address)
    }

    fn get_local_peer_id(&self) -> MockPeerId {
        self.address.into()
    }

    async fn message<M: Message>(
        &self,
        message: M,
        peer_id: MockPeerId,
    ) -> Result<(), RequestError> {
        self.request_impl(message, peer_id).await
    }

    async fn request<Req: Request>(
        &self,
        request: Req,
        peer_id: MockPeerId,
    ) -> Result<Req::Response, RequestError> {
        self.request_impl(request, peer_id).await
    }

    fn receive_messages<M: Message>(&self) -> BoxStream<'static, (M, Self::PeerId)> {
        self.receive_requests_impl()
            .map(|(message, _, sender)| (message, sender))
            .boxed()
    }

    fn receive_requests<Req: Request>(
        &self,
    ) -> BoxStream<'static, (Req, Self::RequestId, Self::PeerId)> {
        self.receive_requests_impl()
    }

    async fn respond<Req: Request>(
        &self,
        request_id: Self::RequestId,
        response: Req::Response,
    ) -> Result<(), Self::Error> {
        let mut hub = self.hub.lock();
        if let Some(responder) = hub.response_senders.remove(&request_id) {
            if !self.peers.read().contains_key(&responder.peer) {
                return Err(MockNetworkError::NotConnected);
            }

            let mut data = Vec::with_capacity(response.serialized_size());
            response.serialize(&mut data).unwrap();

            responder
                .sender
                .send(data)
                .map_err(|_| MockNetworkError::CantRespond(request_id))
        } else {
            Err(MockNetworkError::CantRespond(request_id))
        }
    }

    fn peer_provides_required_services(&self, _peer_id: Self::PeerId) -> bool {
        true
    }

    fn peer_provides_services(&self, _peer_id: Self::PeerId, _services: Services) -> bool {
        true
    }

    fn get_peer_info(&self, peer_id: Self::PeerId) -> Option<PeerInfo> {
        self.peers.read().get(&peer_id).cloned()
    }

    async fn get_peers_by_services(
        &self,
        _services: Services,
        _peers: usize,
    ) -> Result<Vec<Self::PeerId>, MockNetworkError> {
        Ok(self.get_peers())
    }
}
