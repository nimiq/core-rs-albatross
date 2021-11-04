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
use crate::sync::history::cluster::SyncClusterResult;
use crate::sync::history::HistorySync;
use crate::sync::request_component::HistorySyncStream;

impl<TNetwork: Network> HistorySync<TNetwork> {
    /// Poll network events to add/remove peers.
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

    /// Poll ongoing epoch_id requests as long as we don't have too many clusters.
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
                    return Poll::Ready(Some(epoch_ids.sender));
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

    /// Poll the active sync cluster.
    fn poll_cluster(&mut self, cx: &mut Context<'_>) {
        // Initialize active_cluster if there is none.
        if self.active_cluster.is_none() {
            self.active_cluster = self.pop_next_cluster();
        }

        // Poll the active cluster.
        if let Some(cluster) = self.active_cluster.as_mut() {
            while self.queued_push_ops.len() < Self::MAX_QUEUED_PUSH_OPS {
                let result = match cluster.poll_next_unpin(cx) {
                    Poll::Ready(result) => result,
                    Poll::Pending => break,
                };

                log::debug!("Got result from active cluster: {:?}", result);

                match result {
                    Some(Ok(batch_set)) => {
                        let blockchain = Arc::clone(&self.blockchain);
                        let future = async move {
                            debug!(
                                "Processing epoch #{} ({} history items)",
                                batch_set.block.epoch_number(),
                                batch_set.history.len()
                            );
                            let push_result = spawn_blocking(move || {
                                Blockchain::push_history_sync(
                                    blockchain.upgradable_read(),
                                    Block::Macro(batch_set.block),
                                    &batch_set.history,
                                )
                            })
                            .await
                            .expect("blockchain.push_history_sync() should not panic");
                            (SyncClusterResult::from(push_result), None)
                        }
                        .boxed();
                        self.queued_push_ops.push_back(future);
                    }
                    Some(Err(_)) | None => {
                        if let Some(waker) = self.waker.take() {
                            waker.wake();
                        }

                        // Evict the active cluster if it error'd or finished.
                        // TODO Evict Outdated clusters as well?
                        let cluster = self.active_cluster.take().unwrap();
                        let result = match &result {
                            Some(_) => SyncClusterResult::Error,
                            None => SyncClusterResult::NoMoreEpochs,
                        };
                        let future = async move { (result, Some(cluster)) }.boxed();
                        self.queued_push_ops.push_back(future);

                        break;
                    }
                }
            }
        }
    }

    /// Poll the current push operation.
    fn poll_push_op(&mut self, cx: &mut Context<'_>) {
        while let Some(op) = self.queued_push_ops.front_mut() {
            let (result, evicted_cluster) = match op.poll_unpin(cx) {
                Poll::Ready(result) => result,
                Poll::Pending => break,
            };
            self.queued_push_ops.pop_front();

            log::debug!("Push op completed with result: {:?}", result);

            if result != SyncClusterResult::EpochSuccessful {
                let evicted_cluster = evicted_cluster.unwrap();
                self.finish_cluster(&evicted_cluster, result);
            }

            if let Some(waker) = self.waker.take() {
                waker.wake();
            }
        }
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

        self.poll_push_op(cx);

        Poll::Pending
    }
}
