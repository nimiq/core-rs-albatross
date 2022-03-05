use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::task::Waker;

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use parking_lot::RwLock;
use tokio_stream::wrappers::BroadcastStream;

use nimiq_blockchain::Blockchain;
use nimiq_hash::Blake3Hash;
use nimiq_network_interface::prelude::{Network, NetworkEvent, Peer};

use crate::consensus_agent::ConsensusAgent;
use crate::sync::history::cluster::{SyncCluster, SyncClusterResult};
use crate::sync::request_component::HistorySyncStream;

pub(crate) struct EpochIds<TPeer: Peer> {
    pub locator_found: bool,
    pub ids: Vec<Blake3Hash>,
    pub checkpoint_id: Option<Blake3Hash>, // The most recent checkpoint block in the latest epoch.
    pub first_epoch_number: usize,
    pub sender: Arc<ConsensusAgent<TPeer>>,
}

impl<TPeer: Peer> Clone for EpochIds<TPeer> {
    fn clone(&self) -> Self {
        EpochIds {
            locator_found: self.locator_found,
            ids: self.ids.clone(),
            checkpoint_id: self.checkpoint_id.clone(),
            first_epoch_number: self.first_epoch_number,
            sender: Arc::clone(&self.sender),
        }
    }
}

impl<TPeer: Peer> EpochIds<TPeer> {
    pub(crate) fn get_checkpoint_epoch(&self) -> usize {
        self.first_epoch_number + self.ids.len()
    }
}

pub(crate) enum Job<TPeer: Peer> {
    PushBatchSet(usize, Blake3Hash, BoxFuture<'static, SyncClusterResult>),
    FinishCluster(SyncCluster<TPeer>, SyncClusterResult),
}

pub struct HistorySync<TNetwork: Network> {
    pub(crate) blockchain: Arc<RwLock<Blockchain>>,
    pub(crate) network_event_rx: BroadcastStream<NetworkEvent<TNetwork::PeerType>>,
    pub(crate) agents:
        HashMap<Arc<TNetwork::PeerType>, (Arc<ConsensusAgent<TNetwork::PeerType>>, usize)>,
    pub(crate) epoch_ids_stream:
        FuturesUnordered<BoxFuture<'static, Option<EpochIds<TNetwork::PeerType>>>>,
    pub(crate) epoch_clusters: VecDeque<SyncCluster<TNetwork::PeerType>>,
    pub(crate) checkpoint_clusters: VecDeque<SyncCluster<TNetwork::PeerType>>,
    pub(crate) active_cluster: Option<SyncCluster<TNetwork::PeerType>>,
    pub(crate) job_queue: VecDeque<Job<TNetwork::PeerType>>,
    pub(crate) waker: Option<Waker>,
}

pub enum HistorySyncReturn<TPeer: Peer> {
    Good(Arc<ConsensusAgent<TPeer>>),
    Outdated(Arc<ConsensusAgent<TPeer>>),
}

impl<TNetwork: Network> HistorySync<TNetwork> {
    pub(crate) const MAX_CLUSTERS: usize = 100;
    pub(crate) const MAX_QUEUED_JOBS: usize = 4;

    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network_event_rx: BroadcastStream<NetworkEvent<TNetwork::PeerType>>,
    ) -> Self {
        Self {
            blockchain,
            network_event_rx,
            agents: HashMap::new(),
            epoch_ids_stream: FuturesUnordered::new(),
            epoch_clusters: VecDeque::new(),
            checkpoint_clusters: VecDeque::new(),
            active_cluster: None,
            job_queue: VecDeque::new(),
            waker: None,
        }
    }

    pub fn agents(&self) -> impl Iterator<Item = &Arc<ConsensusAgent<TNetwork::PeerType>>> {
        self.agents.values().map(|(agent, _)| agent)
    }

    pub fn remove_agent(&mut self, peer_id: <<TNetwork as Network>::PeerType as Peer>::Id) {
        for cluster in self.epoch_clusters.iter_mut() {
            cluster.remove_peer(&peer_id);
        }
        for cluster in self.checkpoint_clusters.iter_mut() {
            cluster.remove_peer(&peer_id);
        }
        if let Some(cluster) = self.active_cluster.as_mut() {
            cluster.remove_peer(&peer_id);
        }
    }
}

impl<TNetwork: Network> HistorySyncStream<TNetwork::PeerType> for HistorySync<TNetwork> {
    fn add_agent(&self, agent: Arc<ConsensusAgent<TNetwork::PeerType>>) {
        trace!("Requesting epoch ids for peer: {:?}", agent.peer.id());
        let future = Self::request_epoch_ids(Arc::clone(&self.blockchain), agent).boxed();
        self.epoch_ids_stream.push(future);

        // Pushing the future to FuturesUnordered above does not wake the task that
        // polls `epoch_ids_stream`. Therefore, we need to wake the task manually.
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::RwLock;

    use nimiq_blockchain::Blockchain;
    use nimiq_database::volatile::VolatileEnvironment;
    use nimiq_hash::Blake3Hash;
    use nimiq_network_interface::prelude::Network;
    use nimiq_network_mock::{MockHub, MockNetwork, MockPeer};
    use nimiq_primitives::networks::NetworkId;
    use nimiq_utils::time::OffsetTime;

    use crate::consensus_agent::ConsensusAgent;
    use crate::sync::history::sync::EpochIds;
    use crate::sync::history::HistorySync;

    #[tokio::test]
    async fn it_can_cluster_epoch_ids() {
        fn generate_epoch_ids(
            agent: &Arc<ConsensusAgent<MockPeer>>,
            len: usize,
            first_epoch_number: usize,
            diverge_at: Option<usize>,
        ) -> EpochIds<MockPeer> {
            let mut ids = vec![];
            for i in first_epoch_number..first_epoch_number + len {
                let mut epoch_id = [0u8; 32];
                epoch_id[0..8].copy_from_slice(&i.to_le_bytes());

                if diverge_at
                    .map(|d| i >= d + first_epoch_number)
                    .unwrap_or(false)
                {
                    epoch_id[9] = 1;
                }

                ids.push(Blake3Hash::from(epoch_id));
            }

            EpochIds {
                locator_found: true,
                ids,
                checkpoint_id: None,
                first_epoch_number,
                sender: Arc::clone(agent),
            }
        }

        let time = Arc::new(OffsetTime::new());
        let env1 = VolatileEnvironment::new(10).unwrap();
        let blockchain = Arc::new(RwLock::new(
            Blockchain::new(env1, NetworkId::UnitAlbatross, time).unwrap(),
        ));

        let mut hub = MockHub::default();

        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let net3 = Arc::new(hub.new_network());
        net1.dial_mock(&net2);
        net1.dial_mock(&net3);
        let peers = net1.get_peers();
        let consensus_agents: Vec<_> = peers
            .into_iter()
            .map(ConsensusAgent::new)
            .map(Arc::new)
            .collect();

        fn run_test<F>(
            blockchain: &Arc<RwLock<Blockchain>>,
            net: &Arc<MockNetwork>,
            epoch_ids1: EpochIds<MockPeer>,
            epoch_ids2: EpochIds<MockPeer>,
            test: F,
            symmetric: bool,
        ) where
            F: Fn(HistorySync<MockNetwork>),
        {
            let mut sync =
                HistorySync::<MockNetwork>::new(Arc::clone(blockchain), net.subscribe_events());
            sync.cluster_epoch_ids(epoch_ids1.clone());
            sync.cluster_epoch_ids(epoch_ids2.clone());
            test(sync);

            // Symmetric check
            if symmetric {
                let mut sync =
                    HistorySync::<MockNetwork>::new(Arc::clone(blockchain), net.subscribe_events());
                sync.cluster_epoch_ids(epoch_ids2);
                sync.cluster_epoch_ids(epoch_ids1);
                test(sync);
            }
        }

        // This test tests several aspects of the epoch id clustering.
        // 1) identical epoch ids
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 1, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 1);
                assert_eq!(sync.epoch_clusters[0].epoch_ids.len(), 10);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 2);
            },
            true,
        );

        // 2) disjoint epoch ids
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 1, Some(0));
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 2);
                assert_eq!(sync.epoch_clusters[0].epoch_ids.len(), 10);
                assert_eq!(sync.epoch_clusters[1].epoch_ids.len(), 10);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[1].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 1);
                assert_eq!(sync.epoch_clusters[1].batch_set_queue.peers.len(), 1);
            },
            true,
        );

        // 3) same offset and history, second shorter than first
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 8, 1, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 2);
                assert_eq!(sync.epoch_clusters[0].epoch_ids.len(), 8);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 2);
                assert_eq!(sync.epoch_clusters[1].epoch_ids.len(), 2);
                assert_eq!(sync.epoch_clusters[1].first_epoch_number, 9);
                assert_eq!(sync.epoch_clusters[1].batch_set_queue.peers.len(), 1);
            },
            true,
        );

        // 4) different offset, same history, but second is longer
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 3, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 2);
                assert_eq!(sync.epoch_clusters[0].epoch_ids.len(), 10);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 2);
                assert_eq!(sync.epoch_clusters[1].epoch_ids.len(), 2);
                assert_eq!(sync.epoch_clusters[1].first_epoch_number, 11);
                assert_eq!(sync.epoch_clusters[1].batch_set_queue.peers.len(), 1);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change

        // 5) Irrelevant epoch ids (that would constitute forks from what we have already seen.
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 0, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 10, 0, Some(0));
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 0);
            },
            true,
        );

        // 6) different offset, same history, but second is shorter
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 5, 3, None);
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 2);
                assert_eq!(sync.epoch_clusters[0].epoch_ids.len(), 7);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 2);
                assert_eq!(sync.epoch_clusters[1].epoch_ids.len(), 3);
                assert_eq!(sync.epoch_clusters[1].first_epoch_number, 8);
                assert_eq!(sync.epoch_clusters[1].batch_set_queue.peers.len(), 1);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change

        // 7) different offset, diverging history, second longer
        let epoch_ids1 = generate_epoch_ids(&consensus_agents[0], 10, 1, None);
        let epoch_ids2 = generate_epoch_ids(&consensus_agents[1], 8, 4, Some(6));
        run_test(
            &blockchain,
            &net1,
            epoch_ids1,
            epoch_ids2,
            |sync| {
                assert_eq!(sync.epoch_clusters.len(), 3);
                assert_eq!(sync.epoch_clusters[0].epoch_ids.len(), 9);
                assert_eq!(sync.epoch_clusters[0].first_epoch_number, 1);
                assert_eq!(sync.epoch_clusters[0].batch_set_queue.peers.len(), 2);
                assert_eq!(sync.epoch_clusters[1].epoch_ids.len(), 1);
                assert_eq!(sync.epoch_clusters[1].first_epoch_number, 10);
                assert_eq!(sync.epoch_clusters[1].batch_set_queue.peers.len(), 1);
                assert_eq!(sync.epoch_clusters[2].epoch_ids.len(), 2);
                assert_eq!(sync.epoch_clusters[2].first_epoch_number, 10);
                assert_eq!(sync.epoch_clusters[2].batch_set_queue.peers.len(), 1);
            },
            false,
        ); // TODO: for a symmetric check, blockchain state would need to change
    }
}
