use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{stream::BoxStream, StreamExt};
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::Address;
use nimiq_network_interface::{
    network::{Network, NetworkEvent, SubscribeEvents},
    request::{request_handler, Handle},
};
use nimiq_primitives::account::AccountType;
use nimiq_transaction::account::staking_contract::IncomingStakingTransactionData;
use nimiq_utils::spawn;
use parking_lot::RwLock;

use crate::{
    messages::{
        AddressNotification, AddressSubscriptionOperation, AddressSubscriptionTopic,
        NotificationEvent, RequestSubscribeToAddress,
    },
    SubscribeToAddressesError,
    SubscribeToAddressesError::*,
};

/// The max number of peers that can be subscribed.
pub const MAX_SUBSCRIBED_PEERS: usize = 50;
/// The max number of addresses that can be subscribed, per peer.
pub const MAX_SUBSCRIBED_PEERS_ADDRESSES: usize = 250;

impl<N: Network> Handle<N, Arc<RwLock<RemoteEventDispatcherState<N>>>>
    for RequestSubscribeToAddress
{
    fn handle(
        &self,
        peer_id: N::PeerId,
        state: &Arc<RwLock<RemoteEventDispatcherState<N>>>,
    ) -> Result<(), SubscribeToAddressesError> {
        // Requests are handled concurrently, so the limits are checked under the same lock that is
        // used to update the state. Otherwise concurrent requests could all pass the checks.
        let mut state = state.write();

        match self.operation {
            AddressSubscriptionOperation::Subscribe => {
                // If this is a new peer, we need to check if we can attend it
                if !state.contains_peer(&peer_id) && state.number_of_peers() >= MAX_SUBSCRIBED_PEERS
                {
                    return Err(TooManyPeers);
                }

                // We need to check if this peer would end up with too many addresses subscribed to us
                if state.number_of_addresses_after_subscribe(&peer_id, &self.addresses)
                    > MAX_SUBSCRIBED_PEERS_ADDRESSES
                {
                    return Err(TooManyAddresses);
                }

                state.add_addresses(&peer_id, self.addresses.clone());
            }

            AddressSubscriptionOperation::Unsubscribe => {
                // If we don't know this peer, we don't do anything
                if !state.contains_peer(&peer_id) {
                    return Err(InvalidOperation);
                }

                state.remove_addresses(&peer_id, self.addresses.clone());
            }
        }
        Ok(())
    }
}

/// The state that is maintained by the remote event dispatcher:
/// essentially the addresses and peers that are subscribed to us.
pub struct RemoteEventDispatcherState<N: Network> {
    /// HashMap containing a mapping from peers to their interesting addresses
    subscribed_peers: HashMap<N::PeerId, HashSet<Address>>,

    /// Maintains the current list of interesting addresses and the peers that are interested in those addresses
    subscriptions: HashMap<Address, HashSet<N::PeerId>>,
}

impl<N: Network> RemoteEventDispatcherState<N> {
    /// Creates a new state, only state should be needed per client.
    pub fn new() -> Self {
        Self {
            subscribed_peers: HashMap::new(),
            subscriptions: HashMap::new(),
        }
    }

    /// Returns if a peer is part of our state.
    pub fn contains_peer(&self, peer_id: &N::PeerId) -> bool {
        self.subscribed_peers.contains_key(peer_id)
    }

    /// Returns the number of peers that are currently subscribed to us.
    pub fn number_of_peers(&self) -> usize {
        self.subscribed_peers.len()
    }

    /// Returns the number of addresses a peer would be subscribed to after subscribing to the given addresses.
    /// Addresses the peer is already subscribed to, as well as duplicates, are only counted once.
    fn number_of_addresses_after_subscribe(
        &self,
        peer_id: &N::PeerId,
        addresses: &[Address],
    ) -> usize {
        let subscribed = self.subscribed_peers.get(peer_id);
        let new_addresses: HashSet<&Address> = addresses
            .iter()
            .filter(|address| subscribed.is_none_or(|subscribed| !subscribed.contains(*address)))
            .collect();

        subscribed.map_or(0, HashSet::len) + new_addresses.len()
    }

    /// Adds a new address for an specific peer.
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

    /// Removes a peer from the state, used when a peer leaves the network.
    pub fn remove_peer(&mut self, peer_id: &N::PeerId) {
        if let Some(peer_addresses) = self.subscribed_peers.get(peer_id) {
            // Obtain the addresses that are interested to this peer and remove the peer from those addresses.
            peer_addresses.iter().for_each(|address| {
                Self::remove_subscription(&mut self.subscriptions, address, peer_id);
            });
        }
        // Finally remove the peer
        self.subscribed_peers.remove(peer_id);
    }

    /// Remove addresses from an specific peer, if there are no more addresses from this peer we remove it.
    pub fn remove_addresses(&mut self, peer_id: &N::PeerId, addresses: Vec<Address>) {
        if let Some(peer_addresses) = self.subscribed_peers.get_mut(peer_id) {
            addresses.iter().for_each(|address| {
                peer_addresses.remove(address);
                Self::remove_subscription(&mut self.subscriptions, address, peer_id);
            });

            if peer_addresses.is_empty() {
                // If this peer doesn't have any interesting address left, then we just remove it
                self.subscribed_peers.remove(peer_id);
            }
        }
    }

    /// Obtains the peers that are currently subscribed to us.
    pub fn get_peers(&self, address: &Address) -> Option<HashSet<N::PeerId>> {
        self.subscriptions.get(address).cloned()
    }

    /// Removes a peer from the peers that are interested in an address.
    /// The address is dropped once no peer is interested in it anymore, otherwise `subscriptions`
    /// would keep an entry for every address that was ever subscribed to.
    fn remove_subscription(
        subscriptions: &mut HashMap<Address, HashSet<N::PeerId>>,
        address: &Address,
        peer_id: &N::PeerId,
    ) {
        if let Some(peers) = subscriptions.get_mut(address) {
            peers.remove(peer_id);
            if peers.is_empty() {
                subscriptions.remove(address);
            }
        }
    }
}

impl<N: Network> Default for RemoteEventDispatcherState<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// The remote event dispatcher: used to keep track of peers that are subscribed to us
/// because they are interested in different events.
/// This is mainly used by light clients who want to fetch information from Full & History nodes
pub struct RemoteEventDispatcher<N: Network> {
    /// The state that we maintain, with the peers and their interesting addresses
    state: Arc<RwLock<RemoteEventDispatcherState<N>>>,

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
    /// Creates a new `RemoteEventDispatcher` instance and sets up the necessary
    /// event streams to handle incoming address subscription requests and
    /// observe blockchain and network events.
    pub fn new(network: Arc<N>, blockchain: Arc<RwLock<Blockchain>>) -> Self {
        let state = Arc::new(RwLock::new(RemoteEventDispatcherState::new()));

        // Spawn the network receiver that will take care of processing address subscription requests
        let stream = network.receive_requests::<RequestSubscribeToAddress>();

        spawn(request_handler(&network, stream, &Arc::clone(&state)));

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

    /// This is a helper function to determine if we need to create notifications
    fn add_notification_receipts(
        &self,
        address: &Address,
        txn_hash: Blake2bHash,
        block_number: u32,
        peer_receipts: &mut HashMap<N::PeerId, Vec<(Blake2bHash, u32)>>,
    ) {
        if let Some(peers) = self.state.read().get_peers(address) {
            for peer in peers {
                if let Some(receipts) = peer_receipts.get_mut(&peer) {
                    receipts.push((txn_hash.clone(), block_number))
                } else {
                    peer_receipts.insert(peer, vec![(txn_hash.clone(), block_number)]);
                }
            }
        }
    }
}

impl<N: Network> Future for RemoteEventDispatcher<N> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Listen, and process blockchain events
        while let Poll::Ready(Some(event)) = self.blockchain_event_rx.poll_next_unpin(cx) {
            let mut new_blocks = vec![];

            match event {
                BlockchainEvent::Extended(block_hash) => {
                    let block = self
                        .blockchain
                        .read()
                        .get_block(&block_hash, true, None)
                        .expect("Head block not found");

                    new_blocks.push(block);
                }
                BlockchainEvent::Rebranched(_reverted_blocks, adopted_blocks) => {
                    // We don't notify about reverted block, only adopted blocks
                    new_blocks.extend(adopted_blocks.into_iter().map(|(_, block)| block));
                }
                BlockchainEvent::HistoryAdopted(_) => {
                    // In the future we might be interested in other events
                }
                BlockchainEvent::Stored(_block) => {
                    // Stored events are not reported as they are not on the main chain.
                    // If they ever become main chain blocks, they will be reported then with the respective
                    // BlockchainEvent::Rebranched(..)
                }

                BlockchainEvent::EpochFinalized(_)
                | BlockchainEvent::Finalized(_)
                | BlockchainEvent::ProtocolUpgrade(..) => {
                    // These events are not relevant for the notifications, as they are complements to the 'Extended' event,
                    // and don't report new blocks by themselves.
                }
            }
            // This hash map is used to collect all the notifications for a given peer.
            let mut peer_receipts: HashMap<N::PeerId, Vec<(Blake2bHash, u32)>> = HashMap::new();

            // Collect all possible notifications
            for block in new_blocks {
                if let Some(transactions) = block.transactions() {
                    // First collect the list of peers that will be notified
                    for txn in transactions {
                        let txn = txn.get_raw_transaction();

                        // Process transaction sender
                        self.add_notification_receipts(
                            &txn.sender,
                            txn.hash(),
                            block.block_number(),
                            &mut peer_receipts,
                        );

                        // Process transaction recipients
                        self.add_notification_receipts(
                            &txn.recipient,
                            txn.hash(),
                            block.block_number(),
                            &mut peer_receipts,
                        );

                        // Process staking transaction (which are a special case)
                        if txn.recipient_type == AccountType::Staking {
                            // Parse transaction data
                            let proof = IncomingStakingTransactionData::parse(txn).unwrap();

                            if let IncomingStakingTransactionData::AddStake { staker_address } =
                                proof
                            {
                                self.add_notification_receipts(
                                    &staker_address,
                                    txn.hash(),
                                    block.block_number(),
                                    &mut peer_receipts,
                                );
                            }
                            // In the future we might add other staking notifications
                        }
                    }
                }
            }
            // Notify all interested peers
            for (peer_id, receipts) in peer_receipts {
                let network = Arc::clone(&self.network);
                spawn({
                    async move {
                        let _ = network
                            .publish_subtopic::<AddressSubscriptionTopic>(
                                peer_id.to_string(),
                                AddressNotification {
                                    receipts: receipts.clone(),
                                    event: NotificationEvent::BlockchainExtend,
                                },
                            )
                            .await;
                    }
                });
            }
        }

        // Listen and process network events
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            match result {
                Ok(NetworkEvent::PeerLeft(peer_id)) => {
                    // Remove the peer from internal data structures.
                    self.state.write().remove_peer(&peer_id);
                }
                Ok(_) => {}
                Err(_) => return Poll::Pending,
            }
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use nimiq_keys::Address;
    use nimiq_network_interface::request::Handle;
    use nimiq_network_mock::{MockNetwork, MockPeerId};
    use parking_lot::RwLock;

    use super::{RemoteEventDispatcherState, MAX_SUBSCRIBED_PEERS, MAX_SUBSCRIBED_PEERS_ADDRESSES};
    use crate::{
        messages::{AddressSubscriptionOperation, RequestSubscribeToAddress},
        SubscribeToAddressesError::{self, *},
    };

    type State = Arc<RwLock<RemoteEventDispatcherState<MockNetwork>>>;

    fn new_state() -> State {
        Arc::new(RwLock::new(RemoteEventDispatcherState::new()))
    }

    fn address(i: usize) -> Address {
        let mut bytes = [0u8; 20];
        bytes[..8].copy_from_slice(&(i as u64).to_le_bytes());
        Address::from(bytes)
    }

    fn addresses(range: std::ops::Range<usize>) -> Vec<Address> {
        range.map(address).collect()
    }

    fn subscribe(
        state: &State,
        peer: u64,
        addresses: Vec<Address>,
    ) -> Result<(), SubscribeToAddressesError> {
        RequestSubscribeToAddress {
            operation: AddressSubscriptionOperation::Subscribe,
            addresses,
        }
        .handle(MockPeerId(peer), state)
    }

    fn unsubscribe(
        state: &State,
        peer: u64,
        addresses: Vec<Address>,
    ) -> Result<(), SubscribeToAddressesError> {
        RequestSubscribeToAddress {
            operation: AddressSubscriptionOperation::Unsubscribe,
            addresses,
        }
        .handle(MockPeerId(peer), state)
    }

    #[test]
    fn unsubscribe_drops_addresses_without_peers() {
        let state = new_state();
        assert_eq!(subscribe(&state, 1, addresses(0..2)), Ok(()));
        assert_eq!(subscribe(&state, 2, addresses(1..3)), Ok(()));

        // Address 1 is still subscribed to by peer 2, address 0 isn't subscribed to anymore.
        assert_eq!(unsubscribe(&state, 1, addresses(0..2)), Ok(()));
        {
            let state = state.read();
            assert!(!state.contains_peer(&MockPeerId(1)));
            assert_eq!(state.get_peers(&address(0)), None);
            assert_eq!(
                state.get_peers(&address(1)),
                Some(HashSet::from([MockPeerId(2)]))
            );
            assert_eq!(state.subscriptions.len(), 2);
        }

        assert_eq!(unsubscribe(&state, 2, addresses(1..3)), Ok(()));
        let state = state.read();
        assert!(state.subscribed_peers.is_empty());
        assert!(state.subscriptions.is_empty());
    }

    #[test]
    fn remove_peer_drops_addresses_without_peers() {
        let state = new_state();
        assert_eq!(subscribe(&state, 1, addresses(0..2)), Ok(()));
        assert_eq!(subscribe(&state, 2, addresses(1..3)), Ok(()));

        // Address 1 is still subscribed to by peer 2, address 0 isn't subscribed to anymore.
        state.write().remove_peer(&MockPeerId(1));
        {
            let state = state.read();
            assert!(!state.contains_peer(&MockPeerId(1)));
            assert_eq!(state.get_peers(&address(0)), None);
            assert_eq!(
                state.get_peers(&address(1)),
                Some(HashSet::from([MockPeerId(2)]))
            );
            assert_eq!(state.subscriptions.len(), 2);
        }

        state.write().remove_peer(&MockPeerId(2));
        let state = state.read();
        assert!(state.subscribed_peers.is_empty());
        assert!(state.subscriptions.is_empty());
    }

    #[test]
    fn subscription_churn_does_not_grow_state() {
        let state = new_state();

        // Subscribing to fresh addresses and unsubscribing again (or disconnecting) must not leave anything behind.
        for round in 0..10 {
            let start = round * 2 * MAX_SUBSCRIBED_PEERS_ADDRESSES;
            let unsubscribed = addresses(start..start + MAX_SUBSCRIBED_PEERS_ADDRESSES);
            let disconnected = addresses(
                start + MAX_SUBSCRIBED_PEERS_ADDRESSES..start + 2 * MAX_SUBSCRIBED_PEERS_ADDRESSES,
            );

            assert_eq!(subscribe(&state, 1, unsubscribed.clone()), Ok(()));
            assert_eq!(unsubscribe(&state, 1, unsubscribed), Ok(()));

            assert_eq!(subscribe(&state, 2, disconnected), Ok(()));
            state.write().remove_peer(&MockPeerId(2));
        }

        let state = state.read();
        assert!(state.subscribed_peers.is_empty());
        assert!(state.subscriptions.is_empty());
    }

    #[test]
    fn subscribe_enforces_address_limit() {
        let state = new_state();

        // A single request can't exceed the limit.
        assert_eq!(
            subscribe(&state, 1, addresses(0..MAX_SUBSCRIBED_PEERS_ADDRESSES + 1)),
            Err(TooManyAddresses)
        );
        assert!(!state.read().contains_peer(&MockPeerId(1)));
        assert!(state.read().subscriptions.is_empty());

        // Addresses that are already subscribed to, as well as duplicates, only count once.
        assert_eq!(
            subscribe(&state, 1, addresses(0..MAX_SUBSCRIBED_PEERS_ADDRESSES - 1)),
            Ok(())
        );
        let mut request = addresses(0..MAX_SUBSCRIBED_PEERS_ADDRESSES);
        request.push(address(MAX_SUBSCRIBED_PEERS_ADDRESSES - 1));
        assert_eq!(subscribe(&state, 1, request), Ok(()));

        // Further addresses are rejected once the peer has reached the limit.
        assert_eq!(
            subscribe(&state, 1, vec![address(MAX_SUBSCRIBED_PEERS_ADDRESSES)]),
            Err(TooManyAddresses)
        );
        let state = state.read();
        assert_eq!(
            state.subscribed_peers[&MockPeerId(1)].len(),
            MAX_SUBSCRIBED_PEERS_ADDRESSES
        );
        assert_eq!(state.subscriptions.len(), MAX_SUBSCRIBED_PEERS_ADDRESSES);
    }

    #[test]
    fn subscribe_enforces_peer_limit() {
        let state = new_state();
        let max_peers = MAX_SUBSCRIBED_PEERS as u64;

        for peer in 0..max_peers {
            assert_eq!(subscribe(&state, peer, vec![address(0)]), Ok(()));
        }
        assert_eq!(
            subscribe(&state, max_peers, vec![address(0)]),
            Err(TooManyPeers)
        );
        assert_eq!(state.read().number_of_peers(), MAX_SUBSCRIBED_PEERS);

        // Peers that are already subscribed can still subscribe to more addresses.
        assert_eq!(subscribe(&state, 0, vec![address(1)]), Ok(()));

        // Once a peer is fully unsubscribed, a new peer can take its place.
        assert_eq!(unsubscribe(&state, 0, addresses(0..2)), Ok(()));
        assert_eq!(subscribe(&state, max_peers, vec![address(0)]), Ok(()));
    }
}
