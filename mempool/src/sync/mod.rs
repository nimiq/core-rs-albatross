pub(crate) mod messages;
mod tests;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use futures::{future::BoxFuture, FutureExt, Stream, StreamExt};
use messages::{
    MempoolTransactionType, RequestMempoolHashes, RequestMempoolTransactions,
    ResponseMempoolHashes, ResponseMempoolTransactions,
};
use nimiq_blockchain::Blockchain;
use nimiq_consensus::{sync::sync_interface::SyncEvent, ConsensusProxy};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_network_interface::{
    network::Network,
    peer_info::Services,
    request::{request_handler, RequestError},
};
use nimiq_time::{sleep_until, Sleep};
use nimiq_transaction::{historic_transaction::RawTransactionHash, Transaction};
use nimiq_utils::{spawn, stream::FuturesUnordered};
use parking_lot::RwLock;
use tokio_stream::wrappers::BroadcastStream;

use crate::{executor::PubsubIdOrPeerId, mempool_state::MempoolState};

const MAX_HASHES_PER_REQUEST: usize = 500;
const MAX_TOTAL_HASHES: usize = 25_000;
const SHUTDOWN_TIMEOUT_DURATION: Duration = Duration::from_secs(10 * 60); // 10 minutes

/// Struct responsible for discovering hashes and retrieving corresponding transactions from the mempool of other peers
pub(crate) struct MempoolSyncer<N: Network> {
    /// Timeout to gracefully shutdown the mempool syncer entirely
    shutdown_timer: Pin<Box<Sleep>>,

    /// Consensus sync event receiver used to receive notifications when new peers getting added to live sync
    sync_event_rx: BroadcastStream<SyncEvent<<N>::PeerId>>,

    blockchain: Arc<RwLock<Blockchain>>,

    /// Requests to other peers for fetching transaction hashes that currently are in their mempool
    hashes_requests: FuturesUnordered<
        BoxFuture<'static, (N::PeerId, Result<ResponseMempoolHashes, RequestError>)>,
    >,

    /// Reference to the network in order to send requests
    network: Arc<N>,

    /// Peers with a mempool we reach out for to discover and retrieve their mempool hashes and transactions
    peers: HashSet<N::PeerId>,

    /// The mempool state: the data structure where the transactions are stored locally
    mempool_state: Arc<RwLock<MempoolState>>,

    /// Retrieved transactions that are ready to get verified and pushed into our local mempool
    transactions: VecDeque<(Transaction, N::PeerId)>,

    /// Requests to other peers for fetching transactions by their hashes
    transactions_requests: FuturesUnordered<
        BoxFuture<'static, (N::PeerId, Result<ResponseMempoolTransactions, RequestError>)>,
    >,

    /// Collection of transaction hashes not present in the local mempool
    unknown_hashes: HashMap<Blake2bHash, HashSet<N::PeerId>>,

    /// Collection that keeps track which transactions are currently being requested and to which peer
    requested_transactions: HashMap<N::PeerId, HashMap<Blake2bHash, HashSet<N::PeerId>>>,

    /// The type of mempool transactions requested to other peers
    mempool_transaction_type: MempoolTransactionType,
}

impl<N: Network> MempoolSyncer<N> {
    pub fn new(
        peers: Vec<N::PeerId>,
        transaction_type: MempoolTransactionType,
        blockchain: Arc<RwLock<Blockchain>>,
        consensus: ConsensusProxy<N>,
        mempool_state: Arc<RwLock<MempoolState>>,
    ) -> Self {
        let mut syncer = Self {
            shutdown_timer: Box::pin(sleep_until(Instant::now() + SHUTDOWN_TIMEOUT_DURATION)),
            sync_event_rx: consensus.subscribe_sync_events(),
            blockchain,
            hashes_requests: FuturesUnordered::new(),
            network: consensus.network,
            peers: HashSet::new(),
            mempool_state: Arc::clone(&mempool_state),
            unknown_hashes: HashMap::new(),
            requested_transactions: HashMap::new(),
            transactions: VecDeque::new(),
            transactions_requests: FuturesUnordered::new(),
            mempool_transaction_type: transaction_type,
        };

        for peer_id in peers {
            syncer.add_peer(peer_id);
        }
        debug!(num_peers = %syncer.peers.len(), transaction_type = ?syncer.mempool_transaction_type, "Fetching mempool hashes from peers");
        syncer
    }

    /// Push newly discovered hashes into the `unknown_hashes` and keep track which peers have those hashes
    fn push_unknown_hashes(&mut self, hashes: Vec<Blake2bHash>, peer_id: N::PeerId) {
        let blockchain = self.blockchain.read();
        let state = self.mempool_state.read();

        debug!(peer_id = %peer_id, num = %hashes.len(), "Received unknown mempool hashes");
        for hash in hashes.into_iter().take(MAX_TOTAL_HASHES) {
            // Perform some basic checks to reduce the amount of transactions that are going to be requested later
            if state.contains(&hash)
                || blockchain
                    .contains_tx_in_validity_window(&RawTransactionHash::from((hash).clone()), None)
            {
                continue;
            }

            self.unknown_hashes.entry(hash).or_default().insert(peer_id);
        }
    }

    /// Add peer to discover its mempool.
    /// Adding the peer fails if the peer doesn't provide the `Mempool` network service or the syncer already knows this peer
    fn add_peer(&mut self, peer_id: N::PeerId) {
        if self.peers.contains(&peer_id)
            || !self
                .network
                .peer_provides_services(peer_id, Services::MEMPOOL)
        {
            return;
        }

        trace!(%peer_id, "Peer added to mempool sync");
        self.peers.insert(peer_id);
        let network = Arc::clone(&self.network);
        let transaction_type = self.mempool_transaction_type.clone();
        let request = async move {
            (
                peer_id,
                Self::request_mempool_hashes(network, peer_id, transaction_type).await,
            )
        }
        .boxed();

        self.hashes_requests.push(request);
    }

    /// Create a batch of transaction hashes that haven't yet been requested
    fn batch_hashes_by_peer_id(
        &mut self,
        peer_id: N::PeerId,
    ) -> Option<(RequestMempoolTransactions, N::PeerId)> {
        let hashes: Vec<Blake2bHash> = self
            .unknown_hashes
            .iter()
            .take(MAX_HASHES_PER_REQUEST)
            .filter(|(_, peer_ids)| peer_ids.contains(&peer_id))
            .map(|(hash, _)| {
                // Get a fresh copy of all the other peers who shared to also have the corresponding transaction.
                // These peers act as a fallback in the case where the current peer fails provide the actual transaction
                // so it can get requested via another peer.
                let mut fallback_peers = self.unknown_hashes.get(hash).unwrap().clone();

                // Remove the peer we are about to request the transaction from to not let it
                // be a fallback peer when the request fails.
                fallback_peers.remove(&peer_id);

                self.requested_transactions
                    .entry(peer_id)
                    .or_default()
                    .insert(hash.clone(), fallback_peers);
                hash.to_owned()
            })
            .collect();

        if hashes.is_empty() {
            return None;
        }

        // Tracking issue for HashMap/Set drain_filter: https://github.com/rust-lang/rust/issues/59618.
        // Once stabilized, this second iteration isn't necessary anymore.
        for hash in hashes.iter() {
            self.unknown_hashes.remove(hash);
        }

        debug!(peer_id = %peer_id, num = %hashes.len(), "Fetching mempool transactions from peer");
        Some((RequestMempoolTransactions { hashes }, peer_id))
    }

    /// Spawn request handlers in order to process network responses
    pub fn init_network_request_receivers(
        network: Arc<N>,
        mempool_state: Arc<RwLock<MempoolState>>,
    ) {
        // Spawn the request handler for RequestMempoolHashes responses as a task
        let fut = request_handler(
            &network,
            network.receive_requests::<RequestMempoolHashes>(),
            &mempool_state,
        )
        .boxed();
        spawn(fut);

        // Spawn the request handler for RequestMempoolTransactions responses as a task
        let fut = request_handler(
            &network,
            network.receive_requests::<RequestMempoolTransactions>(),
            &mempool_state,
        )
        .boxed();
        spawn(fut);
    }

    /// While there still are unknown transaction hashes which are not part of a request, generate requests and send them to other peers
    fn send_mempool_transactions_requests(&mut self) {
        if self.unknown_hashes.is_empty() {
            return;
        }

        let mut prepared_requests = vec![];
        while !self.unknown_hashes.is_empty() {
            let peer_ids = self.peers.clone();
            for peer_id in peer_ids {
                if let Some(request) = self.batch_hashes_by_peer_id(peer_id) {
                    prepared_requests.push(request);
                }
            }
        }

        let requests = prepared_requests.into_iter().map(|(request, peer_id)| {
            let network = Arc::clone(&self.network);
            async move {
                (
                    peer_id,
                    Self::request_mempool_transactions(network, peer_id, request.hashes.to_owned())
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
            return Poll::Ready(None);
        }

        // Then we check if peers got added to live sync in the meantime
        while let Poll::Ready(Some(sync_event_stream)) = self.sync_event_rx.poll_next_unpin(cx) {
            match sync_event_stream {
                Ok(event) => match event {
                    SyncEvent::AddLiveSync(peer_id) => self.add_peer(peer_id),
                },
                Err(error) => error!(%error, "Failed to poll consensus sync events"),
            }
        }

        // Then we check our RequestMempoolHashes responses
        while let Poll::Ready(Some((peer_id, result))) = self.hashes_requests.poll_next_unpin(cx) {
            match result {
                Ok(response) => self.push_unknown_hashes(response.hashes, peer_id),
                Err(err) => {
                    error!(%err, %peer_id, "Failed to fetch mempool hashes");
                    self.peers.remove(&peer_id);
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
                Ok(response) => {
                    let mut requested_hashes_from_peer =
                        self.requested_transactions.get_mut(&peer_id);

                    let transactions: Vec<(Transaction, N::PeerId)> = response
                        .transactions
                        .into_iter()
                        .filter_map(|txn| {
                            // Compute the hash for the received transaction and remove it from the peer's collection
                            if let Some(hashes_by_peer) = &mut requested_hashes_from_peer {
                                if hashes_by_peer.remove(&txn.hash()).is_some() {
                                    return Some((txn, peer_id));
                                }
                            }
                            None
                        })
                        .collect();

                    if transactions.is_empty() {
                        continue;
                    }
                    info!(num = %transactions.len(), "Synced mempool transactions");
                    self.transactions.extend(transactions);
                }
                Err(err) => {
                    error!(%err, %peer_id, "Failed to fetch mempool transactions");
                    self.peers.remove(&peer_id);

                    if let Some(failed_transactions) = self.requested_transactions.remove(&peer_id)
                    {
                        for (hash, fallback_peers) in failed_transactions {
                            // Don't retry transaction when there is no known peer for it
                            if fallback_peers.is_empty() {
                                continue;
                            }

                            // Move all the transactions hashes that we're supposed to be retrieved from this peer back to `unknown_hashes`
                            // in order to request them via another peer
                            self.unknown_hashes.insert(hash, fallback_peers);
                        }
                        self.send_mempool_transactions_requests();
                    }
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
