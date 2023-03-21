use beserial::{Deserialize, Serialize};
use futures::stream::BoxStream;
use futures::StreamExt;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_network_interface::network::{NetworkEvent, SubscribeEvents, Topic};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_interface::BlockchainEvent;
use nimiq_keys::Address;
use nimiq_network_interface::request::Handle;
use nimiq_network_interface::{network::Network, request::request_handler};

use crate::messages::RequestSubscribeToAddress;
use crate::messages::{AddressSubscriptionOperation, ResponseSubscribeToAddress};
use crate::SubscribeToAdressesError::*;

//The max number of peers that can be subscribed.
pub const MAX_SUBSCRIBED_PEERS: usize = 5;
//The max number of addresses that can be subscribed, per peer
pub const MAX_SUBSCRIBED_PEERS_ADDRESSES: usize = 10;

/// Different kind of events that could generate notifications
#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
#[repr(u8)]
pub enum NotificationEvent {
    BlockchainExtend,
}

/// Interesting Addresses Notifications:
/// A collection of transaction receipts that might be interesting for some peer
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddressNotification {
    /// The Event that generated this notification
    pub event: NotificationEvent,
    /// Tuples of `(transaction_hash, block_number)`
    #[beserial(len_type(u16, limit = 128))]
    pub receipts: Vec<(Blake2bHash, u32)>,
}

/// Topic used to notify peers about transaction adddresses they are subscribed to
/// The final notification is sent over a subtopic derived from this one, which is specific to each peer
#[derive(Clone, Debug, Default)]
pub struct AddressSubscriptionTopic;

impl Topic for AddressSubscriptionTopic {
    type Item = AddressNotification;

    const BUFFER_SIZE: usize = 1024;
    const NAME: &'static str = "transactions";
    const VALIDATE: bool = true;
}

impl<N: Network> Handle<N, ResponseSubscribeToAddress, Arc<RwLock<RemoteEventDispatcherState<N>>>>
    for RequestSubscribeToAddress
{
    fn handle(
        &self,
        peer_id: N::PeerId,
        state: &Arc<RwLock<RemoteEventDispatcherState<N>>>,
    ) -> ResponseSubscribeToAddress {
        match self.operation {
            AddressSubscriptionOperation::Subscribe => {
                if let Some(peer_addreses) = state.read().subscribed_peers.get(&peer_id) {
                    // We need to check if this peer already has too many addresses subscribed to us
                    if peer_addreses.len() > MAX_SUBSCRIBED_PEERS_ADDRESSES {
                        return ResponseSubscribeToAddress {
                            result: Err(TooManyAddresses),
                        };
                    }
                } else {
                    // If this is a new peer, we need to check if we can attend it
                    if state.read().number_of_peers() > MAX_SUBSCRIBED_PEERS {
                        return ResponseSubscribeToAddress {
                            result: Err(TooManyPeers),
                        };
                    }
                }

                state
                    .write()
                    .add_addresses(&peer_id, self.addresses.clone());
            }

            AddressSubscriptionOperation::Unsubscribe => {
                //If we don't know this peer, we don't do anything
                if !state.read().contains_peer(&peer_id) {
                    return ResponseSubscribeToAddress {
                        result: Err(InvalidOperation),
                    };
                }

                state
                    .write()
                    .remove_addresses(&peer_id, self.addresses.clone());
            }
        }
        ResponseSubscribeToAddress { result: Ok(()) }
    }
}

pub struct RemoteEventDispatcherState<N: Network> {
    // HashMap containing a mapping from peers to their interesting addresses
    pub subscribed_peers: HashMap<N::PeerId, HashSet<Address>>,

    /// Mantains the current list of interesting addresses and the peers that are interested in those addresses
    pub subscriptions: HashMap<Address, HashSet<N::PeerId>>,
}

impl<N: Network> RemoteEventDispatcherState<N> {
    pub fn new() -> Self {
        Self {
            subscribed_peers: HashMap::new(),
            subscriptions: HashMap::new(),
        }
    }

    pub fn contains_peer(&self, peer_id: &N::PeerId) -> bool {
        self.subscribed_peers.contains_key(peer_id)
    }

    pub fn number_of_peers(&self) -> usize {
        self.subscribed_peers.len()
    }

    pub fn add_addresses(&mut self, peer_id: &N::PeerId, addresses: Vec<Address>) {
        // If we already knew this peer, then we just update its interesting addresses
        if let Some(interesting_addresses) = self.subscribed_peers.get_mut(peer_id) {
            interesting_addresses.extend(addresses.iter().cloned())
        } else {
            // Otherwise, we insert a new entry for this peer
            self.subscribed_peers
                .insert(*peer_id, HashSet::from_iter(addresses.iter().cloned()));
        }

        // Now we update our address mapping
        for address in addresses {
            if let Some(peers) = self.subscriptions.get_mut(&address) {
                peers.insert(*peer_id);
            } else {
                let mut new_peers = HashSet::new();
                new_peers.insert(*peer_id);
                self.subscriptions.insert(address, new_peers);
            }
        }
    }

    pub fn remove_peer(&mut self, peer_id: &N::PeerId) {
        if let Some(peer_addresses) = self.subscribed_peers.get(peer_id) {
            // Obtain the addresses that are interested to this peer and remove the peer from those addresses.
            for address in peer_addresses {
                if let Some(peers) = self.subscriptions.get_mut(address) {
                    peers.remove(peer_id);
                }
            }
        }
        // Finally remove the peer
        self.subscribed_peers.remove(peer_id);
    }

    pub fn remove_addresses(&mut self, peer_id: &N::PeerId, addresses: Vec<Address>) {
        if let Some(peer_addresses) = self.subscribed_peers.get_mut(peer_id) {
            // Obtain the addresses that are interested to this peer and remove the ones that are no longer interesting
            for address in &addresses {
                peer_addresses.remove(address);
            }

            if peer_addresses.is_empty() {
                // If this peer doesn't have any interesting address left, then we just remove it
                self.subscribed_peers.remove(peer_id);
            }
        }

        // Remove the peer from the addresses
        for address in addresses {
            if let Some(peers) = self.subscriptions.get_mut(&address) {
                peers.remove(peer_id);
            }
        }
    }

    pub fn get_peers(&self, address: &Address) -> Option<HashSet<N::PeerId>> {
        self.subscriptions.get(address).cloned()
    }
}

impl<N: Network> Default for RemoteEventDispatcherState<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RemoteEventDispatcher<N: Network> {
    /// The state that we mantain, with the peers and their interesting addresses
    pub state: Arc<RwLock<RemoteEventDispatcherState<N>>>,

    /// Blockchain reference, to get blocks from it
    blockchain: Arc<RwLock<Blockchain>>,

    /// Reference to the network, to allow for message validation
    network: Arc<N>,

    /// Stream of blockchain events
    blockchain_event_rx: BoxStream<'static, BlockchainEvent>,

    /// Stream of network events
    network_event_rx: SubscribeEvents<N::PeerId>,
}

impl<N: Network> RemoteEventDispatcher<N> {
    pub fn new(network: Arc<N>, blockchain: Arc<RwLock<Blockchain>>) -> Self {
        let state = Arc::new(RwLock::new(RemoteEventDispatcherState::new()));

        // Spawn the network receiver that will take care of processing address subscription requests
        let stream = network.receive_requests::<RequestSubscribeToAddress>();

        tokio::spawn(request_handler(&network, stream, &Arc::clone(&state)));

        let blockchain_event_rx = blockchain.read().notifier_as_stream();

        let network_events = network.subscribe_events();

        Self {
            state,
            blockchain: Arc::clone(&blockchain),
            network: Arc::clone(&network),
            blockchain_event_rx,
            network_event_rx: network_events,
        }
    }
}

impl<N: Network> Future for RemoteEventDispatcher<N> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Listen, and process blockchain events
        while let Poll::Ready(Some(event)) = self.blockchain_event_rx.poll_next_unpin(cx) {
            match event {
                BlockchainEvent::Extended(block_hash) => {
                    let block = self
                        .blockchain
                        .read()
                        .get_block(&block_hash, true, None)
                        .expect("Head block not found");

                    if let Some(transactions) = block.transactions() {
                        // This hash map is used to collect all the notifications for a given peer.
                        let mut peer_receipts: HashMap<N::PeerId, Vec<(Blake2bHash, u32)>> =
                            HashMap::new();

                        //First collect the list of peers that will be notified
                        for txn in transactions {
                            let txn = txn.get_raw_transaction();

                            // Process transaction senders
                            if let Some(peers) = self.state.read().get_peers(&txn.sender) {
                                for peer in peers {
                                    if let Some(receipts) = peer_receipts.get_mut(&peer) {
                                        receipts.push((txn.hash(), block.block_number()))
                                    } else {
                                        peer_receipts
                                            .insert(peer, vec![(txn.hash(), block.block_number())]);
                                    }
                                }
                            }
                            // Process transaction recipients
                            if let Some(peers) = self.state.read().get_peers(&txn.recipient) {
                                for peer in peers {
                                    if let Some(receipts) = peer_receipts.get_mut(&peer) {
                                        receipts.push((txn.hash(), block.block_number()))
                                    } else {
                                        peer_receipts
                                            .insert(peer, vec![(txn.hash(), block.block_number())]);
                                    }
                                }
                            }
                        }

                        //Notify all interested peers
                        for (peer_id, receipts) in peer_receipts {
                            let network = Arc::clone(&self.network);
                            tokio::spawn({
                                async move {
                                    network
                                        .publish_subtopic::<AddressSubscriptionTopic>(
                                            peer_id.to_string(),
                                            AddressNotification {
                                                receipts: receipts.clone(),
                                                event: NotificationEvent::BlockchainExtend,
                                            },
                                        )
                                        .await
                                        .unwrap();
                                }
                            });
                        }
                    }
                }
                BlockchainEvent::HistoryAdopted(_)
                | BlockchainEvent::Rebranched(_, _)
                | BlockchainEvent::Finalized(_)
                | BlockchainEvent::EpochFinalized(_) => {
                    //TODO: implement the kind of events that might be interesting for other peers
                }
            }
        }

        // Listen and process network events
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            match result {
                Ok(NetworkEvent::PeerLeft(peer_id)) => {
                    // Remove the peer from internal data structures.
                    self.state.write().remove_peer(&peer_id);
                }
                Ok(NetworkEvent::PeerJoined(_peer_id, _)) => {}
                Err(_) => return Poll::Pending,
            }
        }

        Poll::Pending
    }
}
