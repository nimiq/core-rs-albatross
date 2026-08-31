use std::{cmp::min, sync::Arc};

use futures::future::BoxFuture;
use nimiq_network_interface::network::Network;
use nimiq_primitives::{trie::trie_diff::TrieDiff, TreeProof};
use parking_lot::RwLock;
use tokio::sync::Semaphore;

use super::{RequestTrieDiff, ResponseTrieDiff};
use crate::sync::{
    live::block_queue::BlockAndSource,
    peer_list::{PeerList, PeerListIndex},
};

/// Errors that can occur while requesting a trie diff from peers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffRequestError {
    /// The maximum number of diff request attempts was reached.
    MaxTriesExceeded,
}

/// Maximum total number of request attempts per diff request. Peer-list changes may cause the same
/// peer to be selected more than once.
const MAX_TRIES: usize = 15;

/// Handles requesting `TrieDiff` for blocks from peers during live sync.
pub struct DiffRequestComponent<N: Network> {
    network: Arc<N>,
    peers: Arc<RwLock<PeerList<N>>>,
    current_peer_index: PeerListIndex,
    /// Bounds locally active request futures. Dropping a future releases its permit, but cannot
    /// withdraw a request that has already been handed to the network.
    concurrent_requests: Arc<Semaphore>,
}

impl<N: Network> DiffRequestComponent<N> {
    const NUM_PENDING_DIFFS: usize = 5;

    /// Creates a new `DiffRequestComponent` with a limit on concurrent diff requests.
    pub fn new(network: Arc<N>, peers: Arc<RwLock<PeerList<N>>>) -> Self {
        DiffRequestComponent {
            network,
            peers,
            current_peer_index: PeerListIndex::default(),
            concurrent_requests: Arc::new(Semaphore::new(Self::NUM_PENDING_DIFFS)),
        }
    }

    pub fn request_diff(
        &mut self,
    ) -> impl FnMut(&BlockAndSource<N>) -> BoxFuture<'static, Result<TrieDiff, DiffRequestError>> + use<N>
    {
        let mut starting_peer_index = self.current_peer_index.clone();
        self.current_peer_index.increment();

        let peers = Arc::clone(&self.peers);
        let network = Arc::clone(&self.network);
        let concurrent_requests = Arc::clone(&self.concurrent_requests);

        move |(block, block_source)| {
            let peers = Arc::clone(&peers);

            // If we know the peer that sent us this block, we ask them first.
            let mut current_peer_index = peers
                .read()
                .index_of(&block_source.peer_id())
                .unwrap_or_else(|| {
                    starting_peer_index.increment();
                    starting_peer_index.clone()
                });

            let network = Arc::clone(&network);
            let concurrent_requests = Arc::clone(&concurrent_requests);
            let block_desc = format!("{block}");
            let block_hash = block.hash();
            let block_diff_root = block.diff_root().clone();

            Box::pin(async move {
                // Controls the number of concurrent diff requests.
                let _request_permit = concurrent_requests.acquire().await.unwrap();
                let mut num_tries = 0;

                loop {
                    // Get the current peer based on the index.
                    let peer_id = peers.read().get(&current_peer_index);
                    let peer_id = match peer_id {
                        Some(peer_id) => peer_id,
                        None => {
                            // No request can be sent in this iteration, so wait for a peer. A peer
                            // removed after an invalid response is handled by the post-attempt
                            // retry check below before the loop can reach this branch again.
                            let peers_became_nonempty = peers.read().wait_for_peers();
                            if let Some(peers_became_nonempty) = peers_became_nonempty {
                                debug!(block = %block_desc, "couldn't fetch diff: waiting for peers");

                                // This wait is intentionally unbounded: having no peers is not a
                                // failed request attempt. Keep the semaphore permit while parked so
                                // at most `NUM_PENDING_DIFFS` diff requests wait for peers.
                                peers_became_nonempty.await;
                            }
                            continue;
                        }
                    };
                    current_peer_index.increment();

                    // Send the diff request to the selected peer.
                    let result = network
                        .request(
                            RequestTrieDiff {
                                block_hash: block_hash.clone(),
                            },
                            peer_id,
                        )
                        .await;

                    num_tries += 1;

                    match result {
                        // If the peer returns a partial diff, validate it.
                        Ok(ResponseTrieDiff::PartialDiff(diff)) => {
                            // Verify that the returned diff matches the block’s expected diff root.
                            if TreeProof::new(diff.0.iter()).root_hash() == block_diff_root {
                                // If valid, return the diff.
                                return Ok(diff);
                            }
                            // A tree-proof mismatch is cryptographically tied to the response
                            // contents; an honest peer cannot produce it by accident.
                            warn!(%peer_id, block = %block_desc, %num_tries, "couldn't fetch diff: invalid diff, removing peer");
                            peers.write().remove_peer(&peer_id);
                        }
                        Ok(ResponseTrieDiff::IncompleteState) => {
                            debug!(%peer_id, block = %block_desc, %num_tries, "couldn't fetch diff: incomplete state")
                        }
                        Ok(ResponseTrieDiff::UnknownBlockHash) => {
                            debug!(%peer_id, block = %block_desc, %num_tries, "couldn't fetch diff: unknown block hash")
                        }
                        Err(error) => {
                            debug!(%peer_id, block = %block_desc, %num_tries, ?error, "couldn't fetch diff: {}", error)
                        }
                    }

                    // Recompute the limit after processing the response because an invalid diff
                    // may have removed the selected peer. If it was the last peer, `max_tries`
                    // becomes zero and this completed attempt terminates instead of entering the
                    // no-peer waiting branch on the next iteration.
                    let max_tries = min(MAX_TRIES, peers.read().len());
                    if num_tries >= max_tries {
                        debug!(block = %block_desc, %num_tries, %max_tries, "couldn't fetch diff: giving up after maximum tries");
                        return Err(DiffRequestError::MaxTriesExceeded);
                    }
                }
            })
        }
    }

    /// Returns the current peer list used for diff requests.
    pub fn peer_list(&self) -> Arc<RwLock<PeerList<N>>> {
        Arc::clone(&self.peers)
    }
}
