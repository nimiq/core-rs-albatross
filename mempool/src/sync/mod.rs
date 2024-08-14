pub(crate) mod messages;

use std::{
    collections::{
        hash_map::Entry::{Occupied, Vacant},
        HashMap, VecDeque,
    },
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use futures::{
    future::BoxFuture,
    stream::{AbortHandle, Abortable},
    FutureExt, Stream, StreamExt,
};
use messages::{
    MempoolTransactionType, RequestMempoolHashes, RequestMempoolTransactions,
    ResponseMempoolHashes, ResponseMempoolTransactions,
};
use nimiq_blockchain::Blockchain;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_network_interface::{
    network::Network,
    request::{request_handler, RequestError},
};
use nimiq_time::sleep_until;
use nimiq_transaction::{historic_transaction::RawTransactionHash, Transaction};
use nimiq_utils::{spawn, stream::FuturesUnordered};
use parking_lot::RwLock;
use tokio::time::Sleep;

use crate::{executor::PubsubIdOrPeerId, mempool_state::MempoolState};

enum RequestStatus {
    Pending,
    Requested,
    Received,
}

/// Struct for keeping track which peers have a specific hash in their mempool and
/// store if we have a requested the corresponding transaction already to one of those peers.
struct HashRequestStatus<N: Network> {
    status: RequestStatus,
    peer_ids: Vec<N::PeerId>,
}

impl<N: Network> HashRequestStatus<N> {
    pub fn new(peer_ids: Vec<N::PeerId>) -> Self {
        Self {
            status: RequestStatus::Pending,
            peer_ids,
        }
    }

    pub fn add_peer(&mut self, peer_id: N::PeerId) {
        self.peer_ids.push(peer_id);
    }

    pub fn has_peer(&self, peer_id: &N::PeerId) -> bool {
        self.peer_ids.contains(peer_id)
    }

    pub fn is_requested(&self) -> bool {
        matches!(self.status, RequestStatus::Requested)
    }

    pub fn mark_as_requested(&mut self) {
        self.status = RequestStatus::Requested;
    }

    pub fn mark_as_received(&mut self) {
        self.status = RequestStatus::Received
    }
}

const MAX_HASHES_PER_REQUEST: usize = 500;
const MAX_TOTAL_TRANSACTIONS: usize = 5000;
const SHUTDOWN_TIMEOUT_DURATION: Duration = Duration::from_secs(10 * 60); // 10 minutes

/// Struct responsible for discovering hashes and retrieving transactions from the mempool of other nodes that have mempool
pub(crate) struct MempoolSyncer<N: Network> {
    /// Timeout to gracefully shutdown the mempool syncer entirely
    abort_timeout: Pin<Box<Sleep>>,

    /// Blockchain reference
    blockchain: Arc<RwLock<Blockchain>>,

    /// Requests to other peers for fetching transaction hashes that currently are in their mempool
    hashes_requests: FuturesUnordered<
        BoxFuture<'static, (N::PeerId, Result<ResponseMempoolHashes, RequestError>)>,
    >,

    /// Reference to the network in order to send requests
    network: Arc<N>,

    /// Peers with a mempool we reach out for to discover and retrieve their mempool hashes and transactions
    peers: Vec<N::PeerId>,

    /// The mempool state: the data structure where the transactions are stored locally
    mempool_state: Arc<RwLock<MempoolState>>,

    /// Retrieved transactions that are ready to get verified and pushed into our local mempool
    transactions: VecDeque<(Transaction, N::PeerId)>,

    /// Requests to other peers for fetching transactions by their hashes
    transactions_requests: FuturesUnordered<
        BoxFuture<'static, (N::PeerId, Result<ResponseMempoolTransactions, RequestError>)>,
    >,

    /// Collection of transaction hashes we don't have in our local mempool
    unknown_hashes: HashMap<Blake2bHash, HashRequestStatus<N>>,

    /// Abort handle for the hashes request handler
    hashes_request_abort_handle: Option<AbortHandle>,

    /// Abort handle for the transactions request handler
    transactions_request_abort_handle: Option<AbortHandle>,
}

impl<N: Network> MempoolSyncer<N> {
    pub fn new(
        peers: Vec<N::PeerId>,
        transaction_type: MempoolTransactionType,
        network: Arc<N>,
        blockchain: Arc<RwLock<Blockchain>>,
        mempool_state: Arc<RwLock<MempoolState>>,
    ) -> Self {
        let hashes_requests = peers
            .iter()
            .map(|peer_id| {
                let peer_id = *peer_id;
                let network = Arc::clone(&network);
                let transaction_type = transaction_type.to_owned();
                async move {
                    (
                        peer_id,
                        Self::request_mempool_hashes(network, peer_id, transaction_type).await,
                    )
                }
                .boxed()
            })
            .collect();

        debug!(num_peers = %peers.len(), ?transaction_type, "Fetching mempool hashes from peers");

        let mut syncer = Self {
            abort_timeout: Box::pin(sleep_until(Instant::now() + SHUTDOWN_TIMEOUT_DURATION)),
            blockchain,
            hashes_requests,
            network: Arc::clone(&network),
            peers,
            mempool_state: Arc::clone(&mempool_state),
            unknown_hashes: HashMap::new(),
            transactions: VecDeque::new(),
            transactions_requests: FuturesUnordered::new(),
            hashes_request_abort_handle: None,
            transactions_request_abort_handle: None,
        };

        syncer.init_network_request_receivers(&network, &mempool_state);

        syncer
    }

    /// Push newly discovered hashes into the `unknown_hashes` and keep track which peers have those hashes
    fn push_unknown_hashes(&mut self, hashes: Vec<Blake2bHash>, peer_id: N::PeerId) {
        let blockchain = self.blockchain.read();
        let state = self.mempool_state.read();

        debug!(peer_id = %peer_id, num = %hashes.len(), "Received unknown mempool hashes");

        hashes.into_iter().for_each(|hash| {
            // Perform some basic checks to reduce the amount of transactions we are going to request later
            // TODO: what if I respond with MAX_TOTAL_TRANSACTIONS fake hashes
            if self.unknown_hashes.len() < MAX_TOTAL_TRANSACTIONS
                && !blockchain
                    .contains_tx_in_validity_window(&RawTransactionHash::from((hash).clone()), None)
                && !state.contains(&hash)
            {
                match self.unknown_hashes.entry(hash) {
                    Occupied(mut entry) => {
                        entry.get_mut().add_peer(peer_id);
                    }
                    Vacant(entry) => {
                        entry.insert(HashRequestStatus::new(vec![peer_id]));
                    }
                };
            }
        })
    }

    /// Create a batch of transaction hashes that haven't yet been requested
    fn batch_hashes_by_peer_id(
        &mut self,
        peer_id: N::PeerId,
    ) -> Option<(RequestMempoolTransactions, N::PeerId)> {
        let hashes: Vec<Blake2bHash> = self
            .unknown_hashes
            .iter_mut()
            .filter(|(_, request_status)| {
                matches!(request_status.status, RequestStatus::Pending)
                    && request_status.has_peer(&peer_id)
            })
            .take(MAX_HASHES_PER_REQUEST)
            .map(|(hash, request_status)| {
                request_status.mark_as_requested();
                hash.to_owned()
            })
            .collect();

        if hashes.is_empty() {
            // This peer has no interesting transactions for us
            return None;
        }

        debug!(peer_id = %peer_id, num = %hashes.len(), "Fetching mempool transactions from peer");

        Some((RequestMempoolTransactions { hashes }, peer_id))
    }

    /// Spawn request handlers in order to process network responses
    fn init_network_request_receivers(
        &mut self,
        network: &Arc<N>,
        mempool_state: &Arc<RwLock<MempoolState>>,
    ) {
        // Register an abort handle and spawn the request handler for RequestMempoolHashes responses as a task
        let stream = request_handler(
            network,
            network.receive_requests::<RequestMempoolHashes>(),
            mempool_state,
        )
        .boxed();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let future = async move {
            let _ = Abortable::new(stream, abort_registration).await;
        };
        spawn(future);
        self.hashes_request_abort_handle = Some(abort_handle);

        // Register an abort handle and spawn the request handler for RequestMempoolTransactions responses as a task
        let stream = request_handler(
            network,
            network.receive_requests::<RequestMempoolTransactions>(),
            mempool_state,
        )
        .boxed();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let future = async move {
            let _ = Abortable::new(stream, abort_registration).await;
        };
        spawn(future);
        self.transactions_request_abort_handle = Some(abort_handle);
    }

    /// Abort the spawned request handlers
    fn shutdown_request_handlers(&mut self) {
        if let Some(abort_handle) = self.hashes_request_abort_handle.take() {
            abort_handle.abort();
        }

        if let Some(abort_handle) = self.transactions_request_abort_handle.take() {
            abort_handle.abort()
        }
    }

    /// While there still are unknown transaction hashes which are not part of a request, generate requests and send them to other peers
    fn send_mempool_transactions_requests(&mut self) {
        let mut prepared_requests = vec![];

        while self
            .unknown_hashes
            .iter()
            .any(|(_, request_status)| !request_status.is_requested())
        {
            for i in 0..self.peers.len() {
                let peer = self.peers[i];
                if let Some(request) = self.batch_hashes_by_peer_id(peer) {
                    prepared_requests.push(request);
                }
            }
        }

        let requests = prepared_requests.into_iter().map(|request| {
            let peer_id = request.1;
            let network = Arc::clone(&self.network);
            async move {
                (
                    peer_id,
                    Self::request_mempool_transactions(
                        network,
                        peer_id,
                        request.0.hashes.to_owned(),
                    )
                    .await,
                )
            }
            .boxed()
        });

        self.transactions_requests.extend(requests);
    }

    /// Network request for retrieving mempool hashes from other peers
    async fn request_mempool_hashes(
        network: Arc<N>,
        peer_id: N::PeerId,
        transaction_type: MempoolTransactionType,
    ) -> Result<ResponseMempoolHashes, RequestError> {
        network
            .request::<RequestMempoolHashes>(RequestMempoolHashes { transaction_type }, peer_id)
            .await
    }

    /// Network request for retrieving mempool transactions from other peers through a list of provided hashes
    async fn request_mempool_transactions(
        network: Arc<N>,
        peer_id: N::PeerId,
        hashes: Vec<Blake2bHash>,
    ) -> Result<ResponseMempoolTransactions, RequestError> {
        network
            .request::<RequestMempoolTransactions>(RequestMempoolTransactions { hashes }, peer_id)
            .await
    }
}

impl<N: Network> Stream for MempoolSyncer<N> {
    type Item = (Transaction, PubsubIdOrPeerId<N>);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // First we check if we have a result we can yield
        if let Some((transaction, peer_id)) = self.transactions.pop_front() {
            return Poll::Ready(Some((transaction, PubsubIdOrPeerId::PeerId(peer_id))));
        }

        // Then we check if we should shutdown ourself
        if self.abort_timeout.poll_unpin(cx).is_ready() {
            info!("Shutdown mempool syncer");
            self.shutdown_request_handlers();
            return Poll::Ready(None);
        }

        // Then we check our RequestMempoolHashes responses
        while let Poll::Ready(Some((peer_id, result))) = self.hashes_requests.poll_next_unpin(cx) {
            match result {
                Ok(hashes) => {
                    self.push_unknown_hashes(hashes.hashes, peer_id);
                }
                Err(err) => {
                    error!(%err, %peer_id, "Failed to fetch mempool hashes");
                }
            }
        }

        // Then we construct our RequestMempoolTransactions requests and send them over the network to our peers
        self.send_mempool_transactions_requests();

        // Then we check our RequestMempoolTransactions responses
        while let Poll::Ready(Some((peer_id, result))) =
            self.transactions_requests.poll_next_unpin(cx)
        {
            match result {
                Ok(transactions) => {
                    let transactions: Vec<(Transaction, N::PeerId)> = transactions
                        .transactions
                        .into_iter()
                        .filter_map(|txn| {
                            // Filter out transactions we didn't ask and mark the ones we did ask for as received
                            match self.unknown_hashes.entry(txn.hash()) {
                                Occupied(mut entry) => match entry.get().status {
                                    RequestStatus::Requested => {
                                        entry.get_mut().mark_as_received();
                                        Some((txn, peer_id))
                                    }
                                    RequestStatus::Pending | RequestStatus::Received => None,
                                },
                                Vacant(_) => None,
                            }
                        })
                        .collect();

                    if transactions.is_empty() {
                        continue;
                    }

                    info!(num = %transactions.len(), "Pushed synced mempool transactions into the mempool");
                    self.transactions.extend(transactions);
                }
                Err(err) => {
                    error!(%err, %peer_id, "Failed to fetch mempool transactions");
                }
            }
        }

        // By now it could be that we have some results we can yield, so we try again
        if let Some((transaction, peer_id)) = self.transactions.pop_front() {
            return Poll::Ready(Some((transaction, PubsubIdOrPeerId::PeerId(peer_id))));
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, task::Poll, time::Duration};

    use futures::{poll, StreamExt};
    use nimiq_blockchain::{Blockchain, BlockchainConfig};
    use nimiq_database::mdbx::MdbxDatabase;
    use nimiq_genesis::NetworkId;
    use nimiq_hash::{Blake2bHash, Hash};
    use nimiq_keys::{Address, KeyPair, SecureGenerate};
    use nimiq_network_interface::network::Network;
    use nimiq_network_mock::MockHub;
    use nimiq_test_log::test;
    use nimiq_test_utils::{
        test_rng::test_rng,
        test_transaction::{generate_transactions, TestAccount, TestTransaction},
    };
    use nimiq_time::sleep;
    use nimiq_transaction::Transaction;
    use nimiq_utils::{spawn, time::OffsetTime};
    use parking_lot::RwLock;
    use rand::rngs::StdRng;

    use crate::{
        mempool_state::MempoolState,
        mempool_transactions::TxPriority,
        sync::{
            messages::{
                RequestMempoolHashes, RequestMempoolTransactions, ResponseMempoolHashes,
                ResponseMempoolTransactions,
            },
            MempoolSyncer, MempoolTransactionType, MAX_HASHES_PER_REQUEST,
        },
    };

    #[test(tokio::test)]
    async fn it_can_discover_mempool_hashes_and_get_transactions_from_other_peers() {
        // Generate test transactions
        let num_transactions = MAX_HASHES_PER_REQUEST + 10;
        let mut rng = test_rng(true);
        let mempool_transactions = generate_test_transactions(num_transactions, &mut rng);

        // Turn test transactions into transactions
        let (txns, _): (Vec<Transaction>, usize) =
            generate_transactions(mempool_transactions, true);
        let hashes: Vec<Blake2bHash> = txns.iter().map(|txn| txn.hash()).collect();

        // Create an empty blockchain
        let time = Arc::new(OffsetTime::new());
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let blockchain = Arc::new(RwLock::new(
            Blockchain::new(
                env,
                BlockchainConfig::default(),
                NetworkId::UnitAlbatross,
                time,
            )
            .unwrap(),
        ));

        // Setup empty Mempool State
        let state = Arc::new(RwLock::new(MempoolState::new(100, 100)));

        // Setup network
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        // Dial peer
        net1.dial_mock(&net2);
        let peer_ids = net1.get_peers();

        // Setup streams to respond to requests
        let hash_stream = net2.receive_requests::<RequestMempoolHashes>();
        let txns_stream = net2.receive_requests::<RequestMempoolTransactions>();

        let hashes_repsonse = ResponseMempoolHashes {
            hashes: hashes.clone(),
        };

        let net2_clone = Arc::clone(&net2);
        let hashes_listener_future =
            hash_stream.for_each(move |(_request, request_id, _peer_id)| {
                let test_response = hashes_repsonse.clone();
                let net2 = Arc::clone(&net2_clone);
                async move {
                    net2.respond::<RequestMempoolHashes>(request_id, test_response.clone())
                        .await
                        .unwrap();
                }
            });

        let net2_clone = Arc::clone(&net2);
        let txns_listener_future = txns_stream.for_each(move |(_request, request_id, _peer_id)| {
            let test_response = ResponseMempoolTransactions {
                transactions: txns.clone(),
            };
            let net2 = Arc::clone(&net2_clone);
            async move {
                net2.respond::<RequestMempoolTransactions>(request_id, test_response.clone())
                    .await
                    .unwrap();
            }
        });

        // Spawn the request responders
        spawn(hashes_listener_future);
        spawn(txns_listener_future);

        // Create a new mempool syncer
        let mut syncer = MempoolSyncer::new(
            peer_ids,
            MempoolTransactionType::Regular,
            net1,
            Arc::clone(&blockchain),
            Arc::clone(&state),
        );

        // Poll to send out requests to peers
        let _ = poll!(syncer.next());
        sleep(Duration::from_millis(200)).await;

        // Poll to check request responses
        let _ = poll!(syncer.next());

        // Verify that all the hashes of the test transactions we generated, have been received by the syncer
        hashes.iter().for_each(|hash| {
            assert!(syncer.unknown_hashes.contains_key(hash));
        });
        assert_eq!(syncer.unknown_hashes.len(), num_transactions);

        // Poll to request the transactions and we should have at least 1 transaction by now
        sleep(Duration::from_millis(200)).await;
        match poll!(syncer.next()) {
            Poll::Ready(txn) => assert!(txn.is_some()),
            Poll::Pending => unreachable!(),
        }

        // Verify that we received all the transactions we have asked for
        // num_transactions - 1 here because we pop_front immediately on a Poll::Ready()
        assert_eq!(syncer.transactions.len(), num_transactions - 1);
    }

    #[test(tokio::test)]
    async fn it_ignores_transactions_we_already_have_locally() {
        // Generate test transactions
        let num_transactions = 10;
        let mut rng = test_rng(true);
        let known_test_transactions = generate_test_transactions(num_transactions, &mut rng);

        // Turn test transactions into transactions
        let (known_txns, _): (Vec<Transaction>, usize) =
            generate_transactions(known_test_transactions, true);
        let known_hashes: Vec<Blake2bHash> = known_txns.iter().map(|txn| txn.hash()).collect();

        // Setup empty Mempool State
        let state = Arc::new(RwLock::new(MempoolState::new(100, 100)));

        // Load known hashes into the mempool state
        let mut handle = state.write();
        known_txns.iter().for_each(|txn| {
            handle.regular_transactions.insert(&txn, TxPriority::Medium);
        });
        assert_eq!(handle.regular_transactions.len(), known_hashes.len());
        drop(handle);

        // Create an empty blockchain
        let time = Arc::new(OffsetTime::new());
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        let blockchain = Arc::new(RwLock::new(
            Blockchain::new(
                env,
                BlockchainConfig::default(),
                NetworkId::UnitAlbatross,
                time,
            )
            .unwrap(),
        ));

        // Setup network
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        // Dial peer
        net1.dial_mock(&net2);
        let peer_ids = net1.get_peers();

        // Setup stream to respond to requests
        let hash_stream = net2.receive_requests::<RequestMempoolHashes>();

        let hashes_repsonse = ResponseMempoolHashes {
            hashes: known_hashes.clone(),
        };

        let net2_clone = Arc::clone(&net2);
        let hashes_listener_future =
            hash_stream.for_each(move |(_request, request_id, _peer_id)| {
                let test_response = hashes_repsonse.clone();
                let net2 = Arc::clone(&net2_clone);
                async move {
                    net2.respond::<RequestMempoolHashes>(request_id, test_response.clone())
                        .await
                        .unwrap();
                }
            });

        // Spawn the request responder
        spawn(hashes_listener_future);

        // Create a new mempool syncer
        let mut syncer = MempoolSyncer::new(
            peer_ids,
            MempoolTransactionType::Regular,
            net1,
            Arc::clone(&blockchain),
            Arc::clone(&state),
        );

        // Poll to send out requests to peers
        let _ = poll!(syncer.next());
        sleep(Duration::from_millis(200)).await;

        // Poll to check request responses
        let _ = poll!(syncer.next());

        // All the hashes we received are already in the mempool state so should be skipped
        assert!(syncer.unknown_hashes.is_empty());
    }

    fn generate_test_transactions(num: usize, mut rng: &mut StdRng) -> Vec<TestTransaction> {
        let mut mempool_transactions = vec![];

        for _ in 0..num {
            let kp1 = KeyPair::generate(&mut rng);
            let addr1 = Address::from(&kp1.public);
            let account1 = TestAccount {
                address: addr1,
                keypair: kp1,
            };

            let kp2 = KeyPair::generate(&mut rng);
            let addr2 = Address::from(&kp2.public);
            let account2 = TestAccount {
                address: addr2,
                keypair: kp2,
            };

            let mempool_transaction = TestTransaction {
                fee: 0,
                value: 10,
                recipient: account1,
                sender: account2,
            };
            mempool_transactions.push(mempool_transaction);
        }

        mempool_transactions
    }
}
