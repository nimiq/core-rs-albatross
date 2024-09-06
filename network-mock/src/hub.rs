use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    sync::{atomic::AtomicBool, Arc},
};

use nimiq_network_interface::{peer_info::PeerInfo, request::RequestType};
use parking_lot::{Mutex, RwLock};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::{
    network::{MockNetwork, MockRequestId},
    MockAddress, MockPeerId, ObservableHashMap,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SenderKey {
    pub network_recipient: MockAddress,
    pub sender_peer: MockPeerId,
    pub message_type: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RequestKey {
    pub recipient: MockAddress,
    pub message_type: RequestType,
}

#[derive(Debug)]
pub(crate) struct ResponseSender {
    pub peer: MockPeerId,
    pub sender: oneshot::Sender<Vec<u8>>,
}

#[derive(Debug)]
pub(crate) struct MockTopic {
    /// Subscribed peer list
    peers: HashSet<MockAddress>,

    /// Sender channel for the topic
    pub sender: broadcast::Sender<(Arc<Vec<u8>>, MockPeerId)>,
}

#[derive(Debug, Default)]
pub(crate) struct MockHubInner {
    /// Peer maps of all networks.
    pub peer_maps: HashMap<MockAddress, Arc<RwLock<ObservableHashMap<MockPeerId, PeerInfo>>>>,

    /// Senders for direct message sending
    pub network_senders: HashMap<SenderKey, mpsc::Sender<Vec<u8>>>,

    /// Senders for gossipsub topics
    ///
    /// The data is Arc'd, such that cloning is cheap, and we need only a borrow when we deserialize.
    pub gossipsub_topics: HashMap<String, MockTopic>,

    /// Senders for dispatching requests
    pub request_senders: HashMap<RequestKey, mpsc::Sender<(Vec<u8>, MockRequestId, MockPeerId)>>,

    /// Senders for returning responses
    pub response_senders: HashMap<MockRequestId, ResponseSender>,

    /// Counter for unique request IDs
    pub next_request_id: MockRequestId,

    /// DHT
    pub dht: HashMap<Vec<u8>, Vec<u8>>,

    /// Arcs to `AtomicBool`s for each network if they're connected.
    pub is_connected: HashMap<MockAddress, Arc<AtomicBool>>,
}

impl MockHubInner {
    /// Returns the requested MockTopic.
    pub fn get_topic(&mut self, topic_name: &String) -> Option<&MockTopic> {
        self.gossipsub_topics.get(topic_name)
    }

    /// Subscribe to a MockTopic; if the topic doesn't exist yet, this function creates it.
    /// Return the MockTopic when a new address is inserted into the subscribed peer list.
    pub fn subscribe(
        &mut self,
        topic_name: String,
        address: MockAddress,
    ) -> Option<&mut MockTopic> {
        // Get the topic. If the topic doesn't exist yet, insert it into the topics list.
        let topic = self
            .gossipsub_topics
            .entry(topic_name)
            .or_insert_with(|| MockTopic {
                peers: HashSet::new(),
                sender: broadcast::channel(16).0,
            });

        // Add the peer address to the subscribed peer list.
        if topic.peers.insert(address) {
            Some(topic)
        } else {
            None
        }
    }

    /// Unsubscribe from a MockTopic.
    /// Return 'false' if the topic doesn't exist or if the peer wasn't subscribed to it.
    /// Otherwise, return 'true'
    pub fn unsubscribe(&mut self, topic_name: &String, address: &MockAddress) -> bool {
        if let Some(topic) = self.gossipsub_topics.get_mut(topic_name) {
            // Verify that the peer was actually subscribed to the topic.
            if !topic.peers.remove(address) {
                return false;
            }

            // If there are no more peers left, remove the topic from the list and drop the sender.
            if topic.peers.is_empty() {
                drop(self.gossipsub_topics.remove(topic_name).unwrap());
            }
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Default)]
pub struct MockHub {
    last_address: u64,

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
        log::trace!("New mock network with address={}", address);
        MockNetwork::new(address, Arc::clone(&self.inner))
    }
}
