use std::{pin::Pin, sync::{atomic::{AtomicBool, Ordering}, Arc}};

use async_trait::async_trait;
use futures::stream::{Stream, StreamExt};
use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::broadcast;

use beserial::{Deserialize, Serialize};
use nimiq_network_interface::network::{NetworkEvent, PubsubId, Topic};
use nimiq_network_interface::peer::Peer;
use nimiq_network_interface::{network::Network, peer_map::ObservablePeerMap};

use crate::{hub::MockHubInner, peer::MockPeer, MockAddress, MockPeerId};

#[derive(Error, Debug)]
pub enum MockNetworkError {
    #[error("Serialization error: {0}")]
    Serialization(#[from] beserial::SerializingError),

    #[error("Can't connect to peer: {0}")]
    CantConnect(MockAddress),

    #[error("Network is not connected")]
    NotConnected,
}

#[derive(Debug)]
pub struct MockPubsubId<P> {
	propagation_source: P,
}

impl PubsubId<MockPeerId> for MockPubsubId<MockPeerId> {
    fn propagation_source(&self) -> MockPeerId {
        self.propagation_source.clone()
    }
}

#[derive(Debug)]
pub struct MockNetwork {
    address: MockAddress,
    peers: ObservablePeerMap<MockPeer>,
    hub: Arc<Mutex<MockHubInner>>,
    is_connected: Arc<AtomicBool>,
}

impl MockNetwork {
    pub(crate) fn new(address: MockAddress, hub: Arc<Mutex<MockHubInner>>) -> Self {
        let peers = ObservablePeerMap::default();

        let is_connected = {
            let mut hub = hub.lock();

            // Insert out peer map into global peer maps table
            if hub.peer_maps.insert(address, peers.clone()).is_some() {
                panic!("address/peer_id of MockNetwork must be unique: address={}", address);
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
        let is_new = hub.peer_maps
            .get(&address).ok_or(MockNetworkError::CantConnect(address))?
            .insert(MockPeer {
                network_address: address,
                peer_id: self.address.into(),
                hub: Arc::clone(&self.hub),
            });

        if is_new {
            // Insert peer into out peer list
            self.peers.insert(MockPeer {
                network_address: self.address,
                peer_id: address.into(),
                hub: Arc::clone(&self.hub),
            });

            // Set is_connected flag for this network
            self.is_connected.store(true, Ordering::SeqCst);

            // Set is_connected flag for other network
            let is_connected = hub.is_connected.get(&address).unwrap();
            is_connected.store(true, Ordering::SeqCst);
        }
        else {
            log::trace!("Peers are already connected.");
        }

        Ok(())
    }

    /// Dials another mock network. Might panic if the peers are not in the same hub (i.e. if the address of the
    /// other network doesn't exist in our hub).
    pub fn dial_mock(&self, other: &Self) {
        self.dial_mock_address(other.address).unwrap();
    }

    /// Disconnect from all peers
    pub fn disconnect(&self) {
        let hub = self.hub.lock();

        for peer in self.peers.remove_all() {
            let peer_map = hub.peer_maps.get(&peer.id().into()).unwrap_or_else(|| {
                panic!(
                    "We're connected to a peer that doesn't have a connection to us: our_peer_id={}, their_peer_id={}",
                    self.address,
                    peer.id()
                )
            });
            peer_map.remove(&self.address.into());
        }

        self.is_connected.store(false, Ordering::SeqCst);
    }
}

#[async_trait]
impl Network for MockNetwork {
    type PeerType = MockPeer;
    type AddressType = MockAddress;
    type Error = MockNetworkError;
    type PubsubId = MockPubsubId<MockPeerId>;

    fn get_peer_updates(&self) -> (Vec<Arc<MockPeer>>, broadcast::Receiver<NetworkEvent<MockPeer>>) {
        self.peers.subscribe()
    }

    fn get_peers(&self) -> Vec<Arc<MockPeer>> {
        self.peers.get_peers()
    }

    fn get_peer(&self, peer_id: MockPeerId) -> Option<Arc<MockPeer>> {
        self.peers.get_peer(&peer_id)
    }

    fn subscribe_events(&self) -> broadcast::Receiver<NetworkEvent<MockPeer>> {
        self.get_peer_updates().1
    }

    async fn subscribe<T>(&self, topic: &T) -> Result<Pin<Box<dyn Stream<Item = (T::Item, Self::PubsubId)> + Send>>, Self::Error>
    where
        T: Topic + Sync,
    {
        let mut hub = self.hub.lock();
        let is_connected = Arc::clone(&self.is_connected);

        let stream = hub.get_topic(topic.topic()).subscribe().into_stream().filter_map(move |r| {
            let is_connected = Arc::clone(&is_connected);

            async move {
                if is_connected.load(Ordering::SeqCst) {
                    match r {
                        Ok((data, peer_id)) => match T::Item::deserialize_from_vec(&data) {
                            Ok(item) => return Some((item, peer_id)),
                            Err(e) => log::warn!("Dropped item because deserialization failed: {}", e),
                        },
                        Err(broadcast::RecvError::Closed) => {}
                        Err(broadcast::RecvError::Lagged(_)) => log::warn!("Mock gossipsub channel is lagging"),
                    }
                }
                else {
                    log::debug!("Network not connected: Dropping gossipsub message.");
                }

                None
            }
        });

        Ok(stream
            .map(|(topic, peer_id)| {
                let id = MockPubsubId {
                    propagation_source: peer_id,
                };
                (topic, id)
            })
            .boxed())

     //   Ok(stream.boxed())
    }

    async fn publish<T: Topic>(&self, topic: &T, item: T::Item) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let mut hub = self.hub.lock();

        let topic = topic.topic();
        let data = item.serialize_to_vec();

        log::debug!("Peer {} publishing on topic '{}': {:?}", self.address, topic, item);

        if self.is_connected.load(Ordering::SeqCst) {
            hub.get_topic(topic).send((Arc::new(data), self.address.into())).unwrap();
            Ok(())
        }
        else {
            Err(MockNetworkError::NotConnected)
        }
    }

    async fn validate_message(&self, _id: Self::PubsubId) -> Result<bool, Self::Error> {
        unimplemented!()
    }

    async fn dht_get<K, V>(&self, k: &K) -> Result<Option<V>, Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Deserialize + Send + Sync,
    {
        if self.is_connected.load(Ordering::SeqCst) {
            let hub = self.hub.lock();

            if let Some(data) = hub.dht.get(k.as_ref()) {
                Ok(Some(V::deserialize_from_vec(&data)?))
            } else {
                Ok(None)
            }
        }
        else {
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
        }
        else {
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
}
