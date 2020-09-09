use async_trait::async_trait;
use block_albatross::{Block};
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use hash::Blake2bHash;
use network_interface::prelude::{Network, Peer};
use parking_lot::RwLock;

use std::sync::{Arc, Weak};

use crate::consensus::Consensus;
use crate::consensus_agent::ConsensusAgent;
use crate::error::SyncError;
use crate::messages::RequestBlockHashesFilter;
use crate::sync::sync_queue::SyncQueue;
use crate::sync::SyncProtocol;
use std::marker::PhantomData;

pub struct SyncingCluster<P: Peer> {
    hashes: Vec<Blake2bHash>,
    agents: Vec<Weak<ConsensusAgent<P>>>,
}

impl<P: Peer> SyncingCluster<P> {
    /// Checks whether self is a subset of other.
    fn is_subset(&self, other: &SyncingCluster<P>) -> bool {
        if self.hashes.len() < other.hashes.len() {
            self.hashes
                .iter()
                .zip(other.hashes.iter())
                .all(|(a, b)| a == b)
        } else {
            false
        }
    }
}

#[derive(Default)]
pub struct QuickSyncState {
    established: bool,
}

pub struct QuickSync<N> {
    state: RwLock<QuickSyncState>,
    _network: PhantomData<N>,
}

impl<N> Default for QuickSync<N> {
    fn default() -> Self {
        QuickSync {
            state: Default::default(),
            _network: PhantomData,
        }
    }
}

impl<N: Network> QuickSync<N> {
    const DESIRED_PENDING_SIZE: usize = 500;

    async fn request_hashes(
        weak_agent: Weak<ConsensusAgent<N::PeerType>>,
        locator: Blake2bHash,
    ) -> (Weak<ConsensusAgent<N::PeerType>>, Option<Vec<Blake2bHash>>) {
        if let Some(agent) = Weak::upgrade(&weak_agent) {
            let objects = agent
                .request_blocks(
                    vec![locator],
                    1000, // TODO: Use other value
                    RequestBlockHashesFilter::ElectionOnly,
                )
                .await;

            match objects {
                Ok(objects) => (weak_agent, Some(objects.hashes)),
                _ => (weak_agent, None),
            }
        } else {
            (weak_agent, None)
        }
    }

    /// This function takes a list of consensus agents and their current macro blocks
    /// and clusters them.
    /// The clusters are already sorted by size.
    /// TODO: What should we do with subsets?
    fn cluster_hashes(
        mut responses: Vec<(Weak<ConsensusAgent<N::PeerType>>, Option<Vec<Blake2bHash>>)>,
    ) -> Vec<SyncingCluster<N::PeerType>> {
        let mut clusters = vec![];

        while !responses.is_empty() {
            let (agent, response) = responses.pop().unwrap();
            if let Some(hashes) = response {
                // Create a cluster.
                let mut current_cluster = SyncingCluster {
                    hashes,
                    agents: vec![agent],
                };

                // Find all super/subsets of this cluster (remember our current agent is already removed).
                responses.retain(|(agent, response)| {
                    if let Some(ref other_hashes) = response {
                        if &current_cluster.hashes == other_hashes {
                            current_cluster.agents.push(Weak::clone(agent));
                            return false;
                        }
                    } else {
                        return false;
                    }
                    true
                });

                clusters.push(current_cluster);
            }
        }

        clusters.sort_by_key(|cluster: &SyncingCluster<N::PeerType>| cluster.agents.len());

        clusters
    }

    /// Sync with a single cluster:
    /// 1. Chunk hashes in some way and request them from peers.
    /// Return how far syncing was possible.
    async fn sync_cluster(
        &self,
        cluster: &SyncingCluster<N::PeerType>,
        skip_prefix_len: usize,
        consensus: &Arc<Consensus<N>>,
    ) -> Result<(), Vec<Blake2bHash>> {
        info!(
            "Syncing macro blocks with cluster of length {}",
            cluster.hashes.len()
        );
        let hashes = cluster
            .hashes
            .iter()
            .skip(skip_prefix_len)
            .cloned()
            .collect();
        let agents = cluster
            .agents
            .iter()
            .filter_map(|agent| agent.upgrade())
            .collect();
        let mut sync_queue =
            SyncQueue::new(hashes, agents, Self::DESIRED_PENDING_SIZE, |agent, hash| {
                Box::pin(async move { agent.request_epoch(hash).await })
            });

        let mut successfully_synced = vec![];
        let block_index = skip_prefix_len;
        while let Some(block_result) = sync_queue.next().await {
            match block_result {
                Ok(epoch) => {
                    // Hash mismatch.
                    let actual_hash = epoch.block.hash();
                    if actual_hash != cluster.hashes[block_index] {
                        warn!("Got block with different hash than expected");
                        return Err(successfully_synced);
                    }

                    // Add to blockchain.
                    let result = consensus
                        .blockchain
                        .push_isolated_macro_block(Block::Macro(epoch.block), &epoch.transactions);

                    match result {
                        Ok(_) => successfully_synced.push(actual_hash),
                        Err(e) => {
                            panic!("{:?}", e);
                            warn!("Failed to push block {:?} into blockchain", actual_hash);
                            return Err(successfully_synced);
                        }
                    }
                }
                Err(hash) => {
                    warn!("Failed to retrieve block with hash {:?}", hash);
                    return Err(successfully_synced);
                }
            }
        }
        // We successfully synced with this cluster.
        Ok(())
    }

    /// The macro block sync works as follows:
    /// 1. Request macro block hashes starting at our most recent macro hash from peers.
    /// 2. Cluster those and start syncing with the largest cluster.
    /// 3. Chunk hashes in some way and request them from peers.
    async fn macro_sync(&self, consensus: Arc<Consensus<N>>) -> Result<(), SyncError> {
        // Get latest macro block.
        let locator = consensus.blockchain.macro_head_hash();

        // Get weak pointers to all peers.
        let mut agents: Vec<Weak<ConsensusAgent<N::PeerType>>> = {
            let consensus_state = consensus.state.read();
            consensus_state
                .agents
                .values()
                .map(|agent| Arc::downgrade(agent))
                .collect()
        };

        // Then, request hashes from all peers.
        let hash_requests = agents
            .drain(..)
            .map(|agent| Self::request_hashes(agent, locator.clone()))
            .collect::<FuturesUnordered<_>>();
        let hash_responses = hash_requests.collect::<Vec<_>>().await;

        // Cluster hashes.
        let clusters = Self::cluster_hashes(hash_responses);

        let mut synced_cluster: Option<SyncingCluster<N::PeerType>> = None;
        for mut cluster in clusters {
            // If we already successfully synced a cluster, we only allow extensions thereof.
            let skip_prefix_len = if let Some(ref synced_cluster) = synced_cluster {
                if synced_cluster.is_subset(&cluster) {
                    synced_cluster.hashes.len()
                } else {
                    continue;
                }
            } else {
                0
            };

            // Only sync from the point we already synced to.
            match self
                .sync_cluster(&cluster, skip_prefix_len, &consensus)
                .await
            {
                Ok(()) => synced_cluster = Some(cluster),
                Err(synced_hashes) => {
                    if !synced_hashes.is_empty() {
                        cluster.hashes = synced_hashes;
                        synced_cluster = Some(cluster);
                    }
                }
            }
        }

        if let Some(_) = synced_cluster {
            self.state.write().established = true;
            Ok(())
        } else {
            Err(SyncError::NoValidSyncTarget)
        }
    }
}

#[async_trait]
impl<N: Network + 'static> SyncProtocol<N> for QuickSync<N> {
    async fn perform_sync(&self, consensus: Arc<Consensus<N>>) -> Result<(), SyncError> {
        self.macro_sync(consensus).await;
        Ok(())
    }

    fn is_established(&self) -> bool {
        self.state.read().established
    }
}
