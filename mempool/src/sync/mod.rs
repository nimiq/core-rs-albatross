pub(crate) mod messages;
mod tests;

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
        !matches!(self.status, RequestStatus::Pending)
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
    shutdown_timer: Pin<Box<Sleep>>,

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

    /// Collection of transaction hashes not present in the local mempool
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
            shutdown_timer: Box::pin(sleep_until(Instant::now() + SHUTDOWN_TIMEOUT_DURATION)),
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
        if self.shutdown_timer.poll_unpin(cx).is_ready() {
            info!(
                syncer_type = ?self.mempool_transaction_type,
                "Shutdown mempool syncer"
            );
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
