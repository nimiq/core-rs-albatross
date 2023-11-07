use std::{
    collections::HashMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, Stream, StreamExt};
#[cfg(feature = "full")]
use nimiq_blockchain::{Blockchain, CHUNK_SIZE};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_hash::Blake2bHash;
use nimiq_macros::store_waker;
use nimiq_mmr::mmr::position::leaf_number_to_index;
use nimiq_network_interface::{
    network::{CloseReason, Network},
    request::RequestError,
};
use nimiq_primitives::{policy::Policy, task_executor::TaskExecutor};
use parking_lot::RwLock;

use super::syncer::{ValidityWindowSync, ValidityWindowSyncReturn};
#[cfg(feature = "full")]
use crate::messages::{
    HistoryChunk, RequestHistoryChunk, RequestValidityWindowStart, ValidityWindowStartResponse,
};

/// The Validity Window Syncer is a component that operates between the macro sync and the live sync
/// Its purpose is to sync all the historic transactions that happened during the current validity window
/// This is because, as a validator, we need to enforce the "tx in validity window" check when producing blocks
#[cfg(feature = "full")]
pub struct ValidityWindowSyncer<TNetwork: Network> {
    /// The blockchain reference
    pub(crate) blockchain: Arc<RwLock<Blockchain>>,

    /// Reference to the network
    pub(crate) network: Arc<TNetwork>,

    /// Used to track the validity chunks we have requested on a per peer basis
    pub(crate) peer_requests: HashMap<TNetwork::PeerId, ValidityChunkRequest>,

    /// The stream for validity window start proofs
    pub(crate) validity_window_start: FuturesUnordered<
        BoxFuture<
            'static,
            (
                Result<ValidityWindowStartResponse, RequestError>,
                TNetwork::PeerId,
            ),
        >,
    >,

    /// The stream for validity window history chunk requests
    pub(crate) validity_window_chunks: FuturesUnordered<
        BoxFuture<'static, (Result<HistoryChunk, RequestError>, TNetwork::PeerId)>,
    >,

    /// This is just a list of peers that are already synced
    pub(crate) synced_peers: Vec<TNetwork::PeerId>,

    /// The latest block number towards which the validity window was fully synced.
    pub(crate) synced_validity_start: u32,

    /// Task executor to be compatible with wasm and not wasm environments,
    pub(crate) executor: Box<dyn TaskExecutor + Send + 'static>,

    /// Waker used for the poll next function
    pub(crate) waker: Option<Waker>,
}

/// Struct used to track the history chunk requests that we have made on a per peer basis.
pub struct ValidityChunkRequest {
    /// This corresponds to the block that should be used to verify the proof.
    verifier_block_number: u32,
    /// The root hash that should be used to verify the proof.
    root_hash: Blake2bHash,
    /// The chunk index that was requested.
    chunk_index: u32,
    /// Initial leaf index offset
    initial_offset: u32,
    /// Validity start block number that we are syncing with this peer
    validity_start: u32,
}

#[cfg(feature = "full")]
impl<TNetwork: Network> ValidityWindowSyncer<TNetwork> {
    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TNetwork>,
        executor: impl TaskExecutor + Send + 'static,
    ) -> Self {
        Self {
            blockchain,
            network,
            peer_requests: HashMap::new(),
            validity_window_start: FuturesUnordered::new(),
            validity_window_chunks: FuturesUnordered::new(),
            waker: None,
            executor: Box::new(executor),
            synced_peers: Vec::new(),
            synced_validity_start: 0,
        }
    }

    pub fn peers(&self) -> impl Iterator<Item = &TNetwork::PeerId> {
        self.peer_requests.keys()
    }

    pub fn remove_peer_requests(&mut self, peer_id: TNetwork::PeerId) {
        self.peer_requests.remove(&peer_id);
    }

    pub fn disconnect_peer(&mut self, peer_id: TNetwork::PeerId, reason: CloseReason) {
        // Remove all pending peer requests (if any)
        self.remove_peer_requests(peer_id);
        let network = Arc::clone(&self.network);
        // We disconnect from this peer
        self.executor.exec(Box::pin({
            async move {
                network.disconnect_peer(peer_id, reason).await;
            }
        }));
    }

    pub async fn discover_validity_window_items(
        network: Arc<TNetwork>,
        blockchain: Arc<RwLock<Blockchain>>,
        peer_id: TNetwork::PeerId,
    ) -> Result<ValidityWindowStartResponse, RequestError> {
        let macro_head = blockchain.read().macro_head();

        // Send the request to discover the start of the validity window.
        network
            .request::<RequestValidityWindowStart>(
                RequestValidityWindowStart {
                    macro_head_number: macro_head.block_number(),
                    macro_head_hash: macro_head.hash(),
                },
                peer_id,
            )
            .await
    }

    pub async fn request_validity_window_chunk(
        network: Arc<TNetwork>,
        peer_id: TNetwork::PeerId,
        epoch_number: u32,
        block_number: u32,
        chunk_index: u64,
    ) -> Result<HistoryChunk, RequestError> {
        // A validity window chunk is simply a history chunk
        network
            .request::<RequestHistoryChunk>(
                RequestHistoryChunk {
                    epoch_number,
                    block_number,
                    chunk_index,
                },
                peer_id,
            )
            .await
    }

    fn poll_validity_window_discover_requests(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<ValidityWindowSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some((request_result, peer_id))) =
            self.validity_window_start.poll_next_unpin(cx)
        {
            match request_result {
                Ok(response) => {
                    // We need to verify the validity window start proof
                    if let Some(proof) = response.proof {
                        let macro_head = self.blockchain.read().macro_head();

                        // Calculate the current validity window bn
                        let validity_window_bn = macro_head
                            .block_number()
                            .saturating_sub(Policy::transaction_validity_window_blocks());

                        // This must correspond to a macro block.
                        assert!(Policy::is_macro_block_at(validity_window_bn));

                        // Now we analize and check the transactions that we obtained from the proof:
                        // We verify there are only two txns in the proof
                        if proof.history.len() != 2 {
                            // There could be a special situation where the first transaction in the blockchain
                            // is located after the validity start.
                            // In this case we recieve a proof that contains only that transaction
                            if proof.history.len() == 1 && proof.positions[0] == 1 {
                                if proof.history[0].block_number < validity_window_bn {
                                    return Poll::Ready(Some(ValidityWindowSyncReturn::Outdated(
                                        peer_id,
                                    )));
                                }

                                let verification_result = proof
                                    .verify(macro_head.header.history_root.clone())
                                    .map_or(false, |result| result);
                                if !verification_result {
                                    log::warn!(peer=%peer_id,"Validity start proof didnt verify, disconnecting peer");
                                    self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                                    return Poll::Ready(None);
                                }

                                self.peer_requests.insert(
                                    peer_id,
                                    ValidityChunkRequest {
                                        verifier_block_number: macro_head.block_number(),
                                        root_hash: macro_head.header.history_root.clone(),
                                        chunk_index: 0,
                                        initial_offset: 0,
                                        validity_start: validity_window_bn,
                                    },
                                );

                                let network = Arc::clone(&self.network);

                                self.validity_window_chunks.push(
                                    async move {
                                        (
                                            Self::request_validity_window_chunk(
                                                network,
                                                peer_id,
                                                Policy::epoch_at(proof.history[0].block_number),
                                                macro_head.block_number(),
                                                0,
                                            )
                                            .await,
                                            peer_id,
                                        )
                                    }
                                    .boxed(),
                                );

                                return Poll::Pending;
                            } else {
                                log::warn!("Validity start proof doesn't contain two transactions");
                                self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                                return Poll::Ready(None);
                            }
                        }

                        // Verify the txns leaf indexes are consecutive.
                        let positions = proof.positions.clone();
                        if positions[0] + 1 != positions[1] {
                            log::error!("The validity start transactions are not consecutive");
                            log::error!("Positions: {:?}", positions);
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }

                        // Now we extract the transactions from the proof
                        let txn_proof_bn;

                        // Extract the block numbers that correspond to the transactions
                        let first_txn = proof.history[0].block_number;
                        let second_txn = proof.history[1].block_number;

                        // They sould correspond to different block numbers:
                        // The second_txn in the proof should correspond to the first transaction in the validity start
                        // And the first_txn should correspond to the transaction before,
                        // Which should correspond to a previous block number (if the server is not lying and omitting txns)
                        if first_txn >= second_txn {
                            log::error!("Validity start transactions do not correspond to different block numbers");
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }

                        // The txn that corresponds to a macro block is the one that is considered to belong to the validity window start
                        if Policy::is_macro_block_at(second_txn) {
                            txn_proof_bn = second_txn;
                        } else {
                            // This could be a special case where the validity start doesn't contain any txn
                            // In this case, the proof should contain two transactions:
                            // One before the validity start and one after the validity start.
                            // And there should be a macro block between the two (which is the validity start)
                            if Policy::macro_block_after(first_txn)
                                == Policy::macro_block_before(second_txn)
                            {
                                // This is the validity start
                                txn_proof_bn = Policy::macro_block_before(second_txn);
                            } else {
                                log::warn!(peer=%peer_id, "The validity start proof does not contain a valid format");
                                self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                                return Poll::Ready(None);
                            }
                        }

                        if txn_proof_bn <= Policy::genesis_block_number() {
                            log::warn!(peer=%peer_id, "Received a validity start proof for the genesis block, this is malicious");
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }

                        // We check if the proof corresponds to our validity window start
                        if txn_proof_bn != validity_window_bn {
                            if txn_proof_bn < validity_window_bn {
                                // Peer is outdated so we emit it.
                                return Poll::Ready(Some(ValidityWindowSyncReturn::Outdated(
                                    peer_id,
                                )));
                            } else {
                                log::warn!(peer=%peer_id,"The proof is ahead than our current validity window");
                                self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                                return Poll::Ready(None);
                            }
                        }

                        let next_election = Policy::election_block_after(validity_window_bn);

                        // Now we determine which is the right root to verify the proof
                        let (verifier_block_number, expected_root) =
                            if Policy::is_election_block_at(validity_window_bn) {
                                // If the validity window starts at an election we use the start itself.
                                let election = self
                                    .blockchain
                                    .read()
                                    .get_block_at(validity_window_bn, false, None)
                                    .unwrap();
                                (validity_window_bn, election.history_root().clone())
                            } else if next_election < macro_head.block_number() {
                                // This is the case where we are crossing an election block
                                let election = self
                                    .blockchain
                                    .read()
                                    .get_block_at(next_election, false, None)
                                    .unwrap();
                                (next_election, election.history_root().clone())
                            } else {
                                // We don't have any election in between so we use the macro head
                                (
                                    macro_head.block_number(),
                                    macro_head.header.history_root.clone(),
                                )
                            };

                        let verification_result = proof
                            .verify(expected_root.clone())
                            .map_or(false, |result| result);

                        if !verification_result {
                            log::warn!(peer=%peer_id,"Validity start proof didnt verify, disconnecting peer");
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }

                        // If the proof verified, we proceed to request the first history chunk
                        let epoch_number = Policy::epoch_at(validity_window_bn);

                        // We use the second transaction from the proof as the starting point, which:
                        // Corresponds to the validity start or the first transaction after the validity start
                        let first_leaf_index = proof.positions[1];

                        // Create a new chunk tracker structure for this peer
                        let first_chunk = (first_leaf_index as u32) / (CHUNK_SIZE as u32);

                        self.peer_requests.insert(
                            peer_id,
                            ValidityChunkRequest {
                                verifier_block_number,
                                root_hash: expected_root,
                                chunk_index: first_chunk,
                                initial_offset: first_chunk * CHUNK_SIZE as u32,
                                validity_start: validity_window_bn,
                            },
                        );

                        // Request the first chunk
                        let network = Arc::clone(&self.network);

                        self.validity_window_chunks.push(
                            async move {
                                (
                                    Self::request_validity_window_chunk(
                                        network,
                                        peer_id,
                                        epoch_number,
                                        verifier_block_number,
                                        first_chunk as u64,
                                    )
                                    .await,
                                    peer_id,
                                )
                            }
                            .boxed(),
                        );
                    } else {
                        // If no proof is provided, we need to check if the start of the validity window is the genesis
                        let macro_head = self.blockchain.read().macro_head();

                        let validity_window_bn = macro_head
                            .block_number()
                            .saturating_sub(Policy::transaction_validity_window_blocks());

                        if validity_window_bn <= Policy::genesis_block_number() {
                            // No inclusion proof, nor validity start for the genesis block
                            // So we just proceed to request the first history chunk.
                            let verifier_block_number = macro_head.block_number();
                            let network = Arc::clone(&self.network);
                            self.peer_requests.insert(
                                peer_id,
                                ValidityChunkRequest {
                                    verifier_block_number,
                                    root_hash: macro_head.header.history_root.clone(),
                                    chunk_index: 0,
                                    initial_offset: 0,
                                    validity_start: validity_window_bn,
                                },
                            );

                            self.validity_window_chunks.push(
                                async move {
                                    (
                                        Self::request_validity_window_chunk(
                                            network,
                                            peer_id,
                                            Policy::epoch_at(macro_head.block_number()),
                                            verifier_block_number,
                                            0,
                                        )
                                        .await,
                                        peer_id,
                                    )
                                }
                                .boxed(),
                            );
                        } else {
                            log::error!(peer=?peer_id,"No validity start proof was provided for non genesis block");
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer);
                            return Poll::Ready(None);
                        }
                    }
                }
                Err(_) => todo!(),
            }
        }

        Poll::Pending
    }

    fn poll_validity_window_chunks(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<ValidityWindowSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some((chunk, peer_id))) =
            self.validity_window_chunks.poll_next_unpin(cx)
        {
            let synced_validity_start = self.synced_validity_start;

            let macro_head = self.blockchain.read().macro_head().block_number();
            let current_validity_start =
                macro_head.saturating_sub(Policy::transaction_validity_window_blocks());

            if synced_validity_start == current_validity_start {
                // Already synced
                return Poll::Ready(Some(ValidityWindowSyncReturn::Good(peer_id)));
            }

            match chunk {
                Ok(history_chunk) => {
                    if let Some(chunk) = history_chunk.chunk {
                        let peer_request = self.peer_requests.get_mut(&peer_id).unwrap();
                        let expected_root = &peer_request.root_hash;
                        let verifier_block_number = peer_request.verifier_block_number;

                        let leaf_index = peer_request.chunk_index * (CHUNK_SIZE as u32);

                        // Verify the history chunk
                        let verification_result = chunk
                            .verify(expected_root, leaf_index as usize)
                            .map_or(false, |result| result);

                        if verification_result {
                            Blockchain::extend_validity_sync(
                                self.blockchain.upgradable_read(),
                                Policy::epoch_at(verifier_block_number),
                                &chunk.history,
                            );

                            // We need to check if this is the last chunk:
                            let prover_mmr_size = chunk.proof.proof.mmr_size;

                            // Obtain our number of leaves
                            let leaf_count = self
                                .blockchain
                                .read()
                                .history_store
                                .length_at(verifier_block_number, None)
                                as usize;

                            // Now we need to add the initial offseat
                            let offset = leaf_number_to_index(
                                peer_request.initial_offset as usize + leaf_count,
                            );

                            if prover_mmr_size == offset {
                                log::debug!(perr=%peer_id, "Finished validity syncing with this peer");

                                // Signal the validity start that we are synced with.
                                self.synced_validity_start = peer_request.validity_start;

                                // We are complete so we emit the peer
                                self.remove_peer_requests(peer_id);

                                return Poll::Ready(Some(ValidityWindowSyncReturn::Good(peer_id)));
                            }

                            // Update the peer tracker structure
                            let chunk_index = peer_request.chunk_index + 1;
                            peer_request.chunk_index = chunk_index;

                            let network = Arc::clone(&self.network);

                            // Request the next chunk
                            self.validity_window_chunks.push(
                                async move {
                                    (
                                        Self::request_validity_window_chunk(
                                            network,
                                            peer_id,
                                            Policy::epoch_at(verifier_block_number),
                                            verifier_block_number,
                                            chunk_index as u64,
                                        )
                                        .await,
                                        peer_id,
                                    )
                                }
                                .boxed(),
                            );
                        } else {
                            // If the chunk doesnt verify we disconnect from the peer
                            log::debug!("The chunk didn't verify, disconnecting from peer");
                            self.disconnect_peer(peer_id, CloseReason::MaliciousPeer)
                        }
                    } else {
                        // Treating this as the peer doesnt have anything to sync
                        return Poll::Ready(Some(ValidityWindowSyncReturn::Good(peer_id)));

                        //If the peer didnt provide any History Chunk, we disconnect from the peer.
                        //log::debug!(" The peer didn't provide any chunk, disconnecting from peer");
                        //self.disconnect_peer(peer_id, CloseReason::MaliciousPeer)
                    }
                }
                Err(err) => {
                    log::error!( error=%err, peer=%peer_id, "There was a request error when trying to request a history chunk");
                }
            }
        }

        Poll::Pending
    }
}

#[cfg(feature = "full")]
impl<TNetwork: Network> ValidityWindowSync<TNetwork::PeerId> for ValidityWindowSyncer<TNetwork> {
    fn add_peer(&mut self, peer_id: TNetwork::PeerId) {
        let network = Arc::clone(&self.network);
        let blockchain = Arc::clone(&self.blockchain);

        let synced_validity_start = self.synced_validity_start;

        let macro_head = blockchain.read().macro_head().block_number();
        let current_validity_start =
            macro_head.saturating_sub(Policy::transaction_validity_window_blocks());

        if synced_validity_start == current_validity_start {
            self.synced_peers.push(peer_id);
        } else {
            // In this case we have something to sync
            self.validity_window_start.push(
                async move {
                    (
                        Self::discover_validity_window_items(network, blockchain, peer_id).await,
                        peer_id,
                    )
                }
                .boxed(),
            );
        }

        // Pushing the future to FuturesUnordered above does not wake the task that
        // polls `validity_window_stream`. Therefore, we need to wake the task manually.
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }
}

#[cfg(feature = "full")]
impl<TNetwork: Network> Stream for ValidityWindowSyncer<TNetwork> {
    type Item = ValidityWindowSyncReturn<TNetwork::PeerId>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        store_waker!(self, waker, cx);

        if let Some(peer) = self.synced_peers.pop() {
            return Poll::Ready(Some(ValidityWindowSyncReturn::Good(peer)));
        }

        if let Poll::Ready(o) = self.poll_validity_window_discover_requests(cx) {
            return Poll::Ready(o);
        }

        if let Poll::Ready(o) = self.poll_validity_window_chunks(cx) {
            return Poll::Ready(o);
        }

        Poll::Pending
    }
}

pub struct NullValidityWindowSyncer<TNetwork: Network> {
    pub(crate) peers: Vec<TNetwork::PeerId>,
}

impl<TNetwork: Network> NullValidityWindowSyncer<TNetwork> {
    pub fn new() -> Self {
        Self { peers: Vec::new() }
    }
}

impl<TNetwork: Network> Default for NullValidityWindowSyncer<TNetwork> {
    fn default() -> Self {
        Self::new()
    }
}

impl<TNetwork: Network> ValidityWindowSync<TNetwork::PeerId>
    for NullValidityWindowSyncer<TNetwork>
{
    fn add_peer(&mut self, peer_id: TNetwork::PeerId) {
        self.peers.push(peer_id);
    }
}

impl<TNetwork: Network> Stream for NullValidityWindowSyncer<TNetwork> {
    type Item = ValidityWindowSyncReturn<TNetwork::PeerId>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(peer) = self.peers.pop() {
            return Poll::Ready(Some(ValidityWindowSyncReturn::Good(peer)));
        }

        Poll::Pending
    }
}
