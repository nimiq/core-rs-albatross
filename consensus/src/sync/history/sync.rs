use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::task::Waker;

use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use parking_lot::RwLock;
use tokio_stream::wrappers::BroadcastStream;

use nimiq_blockchain::Blockchain;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::prelude::{Network, NetworkEvent, Peer};

use crate::messages::Checkpoint;
use crate::sync::history::cluster::{SyncCluster, SyncClusterResult};
use crate::sync::request_component::HistorySyncStream;

#[derive(Clone)]
pub(crate) struct EpochIds<TPeer: Peer> {
    pub locator_found: bool,
    pub ids: Vec<Blake2bHash>,
    pub checkpoint: Option<Checkpoint>, // The most recent checkpoint block in the latest epoch.
    pub first_epoch_number: usize,
    pub sender: TPeer::Id,
}

impl<TPeer: Peer> EpochIds<TPeer> {
    pub(crate) fn get_checkpoint_epoch(&self) -> usize {
        self.first_epoch_number + self.ids.len()
    }
}

pub(crate) enum Job<TNetwork: Network> {
    PushBatchSet(usize, Blake2bHash, BoxFuture<'static, SyncClusterResult>),
    FinishCluster(SyncCluster<TNetwork>, SyncClusterResult),
}

pub struct HistorySync<TNetwork: Network> {
    pub(crate) blockchain: Arc<RwLock<Blockchain>>,
    pub(crate) network: Arc<TNetwork>,
    pub(crate) network_event_rx: BroadcastStream<NetworkEvent<TNetwork::PeerType>>,
    pub(crate) peers: HashMap<<<TNetwork as Network>::PeerType as Peer>::Id, usize>,
    pub(crate) epoch_ids_stream:
        FuturesUnordered<BoxFuture<'static, Option<EpochIds<TNetwork::PeerType>>>>,
    pub(crate) epoch_clusters: VecDeque<SyncCluster<TNetwork>>,
    pub(crate) checkpoint_clusters: VecDeque<SyncCluster<TNetwork>>,
    pub(crate) active_cluster: Option<SyncCluster<TNetwork>>,
    pub(crate) job_queue: VecDeque<Job<TNetwork>>,
    pub(crate) waker: Option<Waker>,
}

#[derive(Debug)]
pub enum HistorySyncReturn<TPeer: Peer> {
    Good(TPeer::Id),
    Outdated(TPeer::Id),
}

impl<TNetwork: Network> HistorySync<TNetwork> {
    pub(crate) const MAX_CLUSTERS: usize = 100;
    pub(crate) const MAX_QUEUED_JOBS: usize = 4;

    pub fn new(
        blockchain: Arc<RwLock<Blockchain>>,
        network: Arc<TNetwork>,
        network_event_rx: BroadcastStream<NetworkEvent<TNetwork::PeerType>>,
    ) -> Self {
        Self {
            blockchain,
            network,
            network_event_rx,
            peers: HashMap::new(),
            epoch_ids_stream: FuturesUnordered::new(),
            epoch_clusters: VecDeque::new(),
            checkpoint_clusters: VecDeque::new(),
            active_cluster: None,
            job_queue: VecDeque::new(),
            waker: None,
        }
    }

    pub fn peers(&self) -> impl Iterator<Item = &<<TNetwork as Network>::PeerType as Peer>::Id> {
        self.peers.keys()
    }

    pub fn remove_peer(&mut self, peer_id: <<TNetwork as Network>::PeerType as Peer>::Id) {
        for cluster in self.epoch_clusters.iter_mut() {
            cluster.remove_peer(&peer_id);
        }
        for cluster in self.checkpoint_clusters.iter_mut() {
            cluster.remove_peer(&peer_id);
        }
        if let Some(cluster) = self.active_cluster.as_mut() {
            cluster.remove_peer(&peer_id);
        }
        for job in self.job_queue.iter_mut() {
            if let Job::FinishCluster(ref mut cluster, _) = job {
                cluster.remove_peer(&peer_id);
            }
        }
    }
}

impl<TNetwork: Network> HistorySyncStream<TNetwork::PeerType> for HistorySync<TNetwork> {
    fn add_peer(&self, peer_id: <<TNetwork as Network>::PeerType as Peer>::Id) {
        trace!("Requesting epoch ids for peer: {:?}", peer_id);
        let future = Self::request_epoch_ids(
            Arc::clone(&self.blockchain),
            Arc::clone(&self.network),
            peer_id,
        )
        .boxed();
        self.epoch_ids_stream.push(future);

        // Pushing the future to FuturesUnordered above does not wake the task that
        // polls `epoch_ids_stream`. Therefore, we need to wake the task manually.
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }
}
