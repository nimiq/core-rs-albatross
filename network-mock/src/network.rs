use std::{pin::Pin, sync::Arc};

use async_trait::async_trait;
use futures::stream::{Stream, StreamExt};
use parking_lot::Mutex;
use thiserror::Error;
use tokio::sync::broadcast;

use beserial::{Deserialize, Serialize};
use nimiq_network_interface::network::{NetworkEvent, Topic};
use nimiq_network_interface::peer::Peer;
use nimiq_network_interface::{network::Network, peer_map::ObservablePeerMap};

use crate::{hub::MockHubInner, peer::MockPeer, MockAddress, MockPeerId};

#[derive(Error, Debug)]
pub enum MockNetworkError {
    #[error("Serialization error: {0}")]
    Serialization(#[from] beserial::SerializingError),

    #[error("Can't connect to peer: {0}")]
    CantConnect(MockAddress),
}

#[derive(Debug)]
pub struct MockNetwork {
    address: MockAddress,
    peers: ObservablePeerMap<MockPeer>,
    hub: Arc<Mutex<MockHubInner>>,
}

impl MockNetwork {
    pub(crate) fn new(address: MockAddress, hub: Arc<Mutex<MockHubInner>>) -> Self {
        let peers = ObservablePeerMap::default();

        // Insert out peer map into global peer maps table
        if hub.lock().peer_maps.insert(address, peers.clone()).is_some() {
            panic!("address/peer_id of MockNetwork must be unique: address={}", address);
        }

        Self { address, peers, hub }
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
            .get(&address)
            .ok_or_else(|| MockNetworkError::CantConnect(address))?
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
    }
}

#[async_trait]
impl Network for MockNetwork {
    type PeerType = MockPeer;
    type AddressType = MockAddress;
    type Error = MockNetworkError;

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

    async fn subscribe<T>(&self, topic: &T) -> Result<Pin<Box<dyn Stream<Item = (T::Item, <Self::PeerType as Peer>::Id)> + Send>>, Self::Error>
    where
        T: Topic + Sync,
    {
        let mut hub = self.hub.lock();
        let stream = hub.get_topic(topic.topic()).subscribe().into_stream().filter_map(|r| async move {
            match r {
                Ok((data, peer_id)) => match T::Item::deserialize_from_vec(&data) {
                    Ok(item) => return Some((item, peer_id)),
                    Err(e) => log::warn!("Dropped item because deserialization failed: {}", e),
                },
                Err(broadcast::RecvError::Closed) => {}
                Err(broadcast::RecvError::Lagged(_)) => log::warn!("Mock gossipsub channel is lagging"),
            }
            None
        });

        Ok(stream.boxed())
    }

    async fn publish<T: Topic>(&self, topic: &T, item: T::Item) -> Result<(), Self::Error>
    where
        T: Topic + Sync,
    {
        let mut hub = self.hub.lock();

        let topic = topic.topic();
        let data = item.serialize_to_vec();

        log::debug!("Peer {} publishing on topic '{}': {:?}", self.address, topic, item);

        hub.get_topic(topic).send((Arc::new(data), self.address.into())).unwrap();
        Ok(())
    }

    async fn dht_get<K, V>(&self, k: &K) -> Result<Option<V>, Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Deserialize + Send + Sync,
    {
        let hub = self.hub.lock();

        if let Some(data) = hub.dht.get(k.as_ref()) {
            Ok(Some(V::deserialize_from_vec(&data)?))
        } else {
            Ok(None)
        }
    }

    async fn dht_put<K, V>(&self, k: &K, v: &V) -> Result<(), Self::Error>
    where
        K: AsRef<[u8]> + Send + Sync,
        V: Serialize + Send + Sync,
    {
        let mut hub = self.hub.lock();

        let data = v.serialize_to_vec();
        hub.dht.insert(k.as_ref().to_owned(), data);
        Ok(())
    }

    async fn dial_peer(&self, peer_id: MockPeerId) -> Result<(), Self::Error> {
        self.dial_mock_address(peer_id.into())
    }

    async fn dial_address(&self, address: MockAddress) -> Result<(), Self::Error> {
        self.dial_mock_address(address)
    }
}
