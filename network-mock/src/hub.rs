use std::{collections::HashMap, hash::Hash, sync::Arc};

use futures::channel::mpsc;
use parking_lot::Mutex;
use tokio::sync::broadcast;

use crate::{network::MockNetwork, peer::MockPeer, MockAddress, MockPeerId};
use nimiq_network_interface::peer_map::ObservablePeerMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SenderKey {
    pub network_recipient: MockAddress,
    pub sender_peer: MockPeerId,
    pub message_type: u64,
}

#[derive(Debug, Default)]
pub(crate) struct MockHubInner {
    /// Peer maps of all networks.
    pub peer_maps: HashMap<MockAddress, ObservablePeerMap<MockPeer>>,

    /// Senders for direct message sending
    pub network_senders: HashMap<SenderKey, mpsc::Sender<Vec<u8>>>,

    /// Senders for gossipsub topics
    ///
    /// The data is Arc'd, such that cloning is cheap, and we need only a borrow when we deserialize.
    pub gossipsub_topics: HashMap<String, broadcast::Sender<(Arc<Vec<u8>>, MockPeerId)>>,

    /// DHT
    pub dht: HashMap<Vec<u8>, Vec<u8>>,
}

impl MockHubInner {
    pub fn get_topic(&mut self, topic: String) -> &broadcast::Sender<(Arc<Vec<u8>>, MockPeerId)> {
        self.gossipsub_topics.entry(topic).or_insert_with(|| broadcast::channel(16).0)
    }
}

#[derive(Debug, Default)]
pub struct MockHub {
    last_address: usize,

    inner: Arc<Mutex<MockHubInner>>,
}

impl MockHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_address(&mut self) -> MockAddress {
        self.last_address += 1;
        MockAddress(self.last_address)
    }

    pub fn new_network(&mut self) -> MockNetwork {
        let address = self.new_address();
        self.new_network_with_address(address)
    }

    pub fn new_network_with_address<A: Into<MockAddress>>(&mut self, address: A) -> MockNetwork {
        let address: MockAddress = address.into();
        log::debug!("New mock network with address={}", address);
        MockNetwork::new(address, Arc::clone(&self.inner))
    }
}
