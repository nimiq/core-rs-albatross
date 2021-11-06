use std::pin::Pin;
use std::sync::Arc;

use futures::stream::Stream;
use futures::task::{Context, Poll};
use futures::{FutureExt, StreamExt};
use tokio::task::spawn_blocking;

use nimiq_block::Block;
use nimiq_blockchain::Blockchain;
use nimiq_network_interface::prelude::{Network, NetworkEvent, Peer};

use crate::consensus_agent::ConsensusAgent;
use crate::sync::history::cluster::{SyncCluster, SyncClusterResult};
use crate::sync::history::sync::Job;
use crate::sync::history::HistorySync;
use crate::sync::request_component::HistorySyncStream;

impl<TNetwork: Network> HistorySync<TNetwork> {
    fn poll_network_events(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Arc<ConsensusAgent<TNetwork::PeerType>>>> {
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            match result {
                Ok(NetworkEvent::PeerLeft(peer)) => {
                    // Delete the ConsensusAgent from the agents map, removing the only "persistent"
                    // strong reference to it. There might not be an entry for every peer (e.g. if
                    // it didn't send any epoch ids).
                    // FIXME This doesn't work if we're currently requesting epoch_ids from this peer
                    self.agents.remove(&peer);
                }
                Ok(NetworkEvent::PeerJoined(peer)) => {
                    // Create a ConsensusAgent for the peer that joined and request epoch_ids from it.
                    let agent = Arc::new(ConsensusAgent::new(peer));
                    self.add_agent(agent);
                }
                Err(_) => return Poll::Ready(None),
            }
        }

        Poll::Pending
    }

    fn poll_epoch_ids(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Arc<ConsensusAgent<TNetwork::PeerType>>>> {
        // TODO We might want to not send an epoch_id request in the first place if we're at the
        //  cluster limit.
        while self.epoch_clusters.len() < Self::MAX_CLUSTERS {
            let epoch_ids = match self.epoch_ids_stream.poll_next_unpin(cx) {
                Poll::Ready(Some(epoch_ids)) => epoch_ids,
                _ => break,
            };

            if let Some(epoch_ids) = epoch_ids {
                // The peer might have disconnected during the request.
                // FIXME Check if the peer is still connected

                // If the peer didn't send any locator, we are done with it and emit it.
                if !epoch_ids.locator_found {
                    debug!(
                        "Peer is behind or on different chain: {:?}",
                        epoch_ids.sender.peer.id()
                    );
                    // TODO Mark peer as useless.
                    // FIXME Emit peer here once the consumer understands useless peers.
                    //return Poll::Ready(Some(epoch_ids.sender));
                } else if epoch_ids.ids.is_empty() && epoch_ids.checkpoint_id.is_none() {
                    // We are synced with this peer.
                    debug!(
                        "Finished syncing with peer: {:?}",
                        epoch_ids.sender.peer.id()
                    );
                    // TODO Mark peer as useful
                    return Poll::Ready(Some(epoch_ids.sender));
                }

                self.cluster_epoch_ids(epoch_ids);
            }
        }

        Poll::Pending
    }

    fn poll_cluster(&mut self, cx: &mut Context<'_>) {
        // Initialize active_cluster if there is none.
        if self.active_cluster.is_none() {
            self.active_cluster = self.pop_next_cluster();
        }

        // Poll the active cluster.
        if let Some(cluster) = self.active_cluster.as_mut() {
            while self.job_queue.len() < Self::MAX_QUEUED_JOBS {
                let result = match cluster.poll_next_unpin(cx) {
                    Poll::Ready(result) => result,
                    Poll::Pending => break,
                };

                match result {
                    Some(Ok(batch_set)) => {
                        let blockchain = Arc::clone(&self.blockchain);
                        let future = async move {
                            debug!(
                                "Processing epoch #{} ({} history items)",
                                batch_set.block.epoch_number(),
                                batch_set.history.len()
                            );
                            spawn_blocking(move || {
                                Blockchain::push_history_sync(
                                    blockchain.upgradable_read(),
                                    Block::Macro(batch_set.block),
                                    &batch_set.history,
                                )
                            })
                            .await
                            .expect("blockchain.push_history_sync() should not panic")
                            .into()
                        }
                        .boxed();

                        self.job_queue
                            .push_back(Job::PushBatchSet(cluster.id, future));
                    }
                    Some(Err(_)) | None => {
                        // Evict the active cluster if it error'd or finished.
                        let cluster = self.active_cluster.take().unwrap();
                        let result = match &result {
                            Some(_) => SyncClusterResult::Error,
                            None => SyncClusterResult::NoMoreEpochs,
                        };

                        self.job_queue
                            .push_back(Job::FinishCluster(cluster, result));

                        if let Some(waker) = self.waker.take() {
                            waker.wake();
                        }
                        break;
                    }
                }
            }
        }
    }

    fn poll_job_queue(&mut self, cx: &mut Context<'_>) {
        while let Some(job) = self.job_queue.front_mut() {
            let result = match job {
                Job::PushBatchSet(_, future) => match future.poll_unpin(cx) {
                    Poll::Ready(result) => Some(result),
                    Poll::Pending => break,
                },
                Job::FinishCluster(_, _) => None,
            };

            let job = self.job_queue.pop_front().unwrap();

            match job {
                Job::PushBatchSet(cluster_id, _) => {
                    let result = result.unwrap();

                    log::debug!(
                        "PushBatchSet from cluster_id {} completed with result: {:?}",
                        cluster_id,
                        result
                    );

                    if result != SyncClusterResult::EpochSuccessful {
                        // The push operation failed, therefore the whole cluster is invalid.
                        // Clean out any jobs originating from the failed cluster from the job_queue.
                        // If the cluster isn't active anymore, get the cluster from the
                        // FinishCluster job in the job_queue.
                        let cluster = self.evict_jobs_by_cluster(cluster_id);

                        // If the failed cluster is the still active, we remove it.
                        let cluster = cluster.unwrap_or_else(|| {
                            self.active_cluster
                                .take()
                                .expect("No cluster in job_queue, active_cluster should exist")
                        });
                        assert_eq!(cluster_id, cluster.id);

                        self.finish_cluster(cluster, result);
                    }
                }
                Job::FinishCluster(cluster, result) => {
                    self.finish_cluster(cluster, result);
                }
            }

            if let Some(waker) = self.waker.take() {
                waker.wake();
            }
        }
    }

    fn evict_jobs_by_cluster(
        &mut self,
        cluster_id: usize,
    ) -> Option<SyncCluster<TNetwork::PeerType>> {
        while let Some(job) = self.job_queue.front() {
            let id = match job {
                Job::PushBatchSet(cluster_id, _) => *cluster_id,
                Job::FinishCluster(cluster, _) => cluster.id,
            };
            if id != cluster_id {
                return None;
            }
            let job = self.job_queue.pop_front().unwrap();
            if let Job::FinishCluster(cluster, _) = job {
                return Some(cluster);
            }
        }
        None
    }
}

impl<TNetwork: Network> Stream for HistorySync<TNetwork> {
    type Item = Arc<ConsensusAgent<TNetwork::PeerType>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        store_waker!(self, waker, cx);

        if let Poll::Ready(o) = self.poll_network_events(cx) {
            return Poll::Ready(o);
        }

        if let Poll::Ready(o) = self.poll_epoch_ids(cx) {
            return Poll::Ready(o);
        }

        self.poll_cluster(cx);

        self.poll_job_queue(cx);

        Poll::Pending
    }
}
