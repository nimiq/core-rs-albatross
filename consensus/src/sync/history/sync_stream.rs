use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{FutureExt, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain::Blockchain;
use nimiq_network_interface::network::{Network, NetworkEvent};
use nimiq_utils::WakerExt as _;
use tokio::task::spawn_blocking;

use crate::sync::{
    history::{
        cluster::{SyncCluster, SyncClusterResult},
        sync::Job,
        HistoryMacroSync,
    },
    syncer::{MacroSync, MacroSyncReturn},
};

impl<TNetwork: Network> HistoryMacroSync<TNetwork> {
    fn poll_network_events(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        while let Poll::Ready(Some(result)) = self.network_event_rx.poll_next_unpin(cx) {
            match result {
                Ok(NetworkEvent::PeerLeft(peer_id)) => {
                    // Remove the peer from all data structures.
                    self.remove_peer(peer_id);
                    self.peers.remove(&peer_id);
                }
                Ok(NetworkEvent::PeerJoined(peer_id, _)) => {
                    // Query if that peer provides the necessary services for syncing
                    if self.network.peer_provides_required_services(peer_id) {
                        // Request epoch_ids from the peer that joined.
                        self.add_peer(peer_id);
                    } else {
                        // We can't sync with this peer as it doesn't provide the services that we need.
                        // Emit the peer as incompatible.
                        return Poll::Ready(Some(MacroSyncReturn::Incompatible(peer_id)));
                    }
                }
                Ok(_) => {}
                Err(_) => return Poll::Ready(None),
            }
        }

        Poll::Pending
    }

    fn poll_epoch_ids(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Option<MacroSyncReturn<TNetwork::PeerId>>> {
        // TODO We might want to not send an epoch_id request in the first place if we're at the
        //  cluster limit.
        while self.epoch_clusters.len() < Self::MAX_CLUSTERS {
            let epoch_ids = match self.epoch_ids_stream.poll_next_unpin(cx) {
                Poll::Ready(Some(epoch_ids)) => epoch_ids,
                _ => break,
            };

            if let Some(epoch_ids) = epoch_ids {
                // The peer might have disconnected during the request.
                if !self.network.has_peer(epoch_ids.sender) {
                    continue;
                }

                // If the peer didn't find any of our locators, we are done with it and emit it.
                if !epoch_ids.locator_found {
                    debug!(
                        "Peer is behind or on different chain: {:?}",
                        epoch_ids.sender
                    );
                    return Poll::Ready(Some(MacroSyncReturn::Outdated(epoch_ids.sender)));
                } else if epoch_ids.ids.is_empty() && epoch_ids.checkpoint.is_none() {
                    // We are synced with this peer.
                    debug!("Finished syncing with peer: {:?}", epoch_ids.sender);
                    return Poll::Ready(Some(MacroSyncReturn::Good(epoch_ids.sender)));
                }

                // If the clustering deems a peer useless, it is returned here and we emit it.
                if let Some(agent) = self.cluster_epoch_ids(epoch_ids) {
                    return Poll::Ready(Some(MacroSyncReturn::Outdated(agent)));
                }
            }
        }

        Poll::Pending
    }

    fn poll_cluster(&mut self, cx: &mut Context<'_>) {
        // Initialize active_cluster if there is none.
        if self.active_cluster.is_none() {
            // Wait for all pending jobs to finish first to ensure that blockchain is up to date.
            // XXX This breaks pipelining across clusters.
            if !self.job_queue.is_empty() {
                return;
            }
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
                        let hash = batch_set.block.hash();
                        let blockchain = Arc::clone(&self.blockchain);

                        // Note the fact that the future surrounding the spawn_blocking is created deliberately as
                        // it is not necessarily polled immediately. It must wait until preceding futures have resolved
                        // before the actual push_history_sync call has any chance of succeeding. Thus using the
                        // spawn_blocking as the future is unfeasible.
                        let future = async move {
                            debug!(
                                "Processing epoch #{} ({} history items)",
                                batch_set.block.epoch_number(),
                                batch_set.history.len()
                            );
                            let result = spawn_blocking(move || {
                                Blockchain::push_history_sync(
                                    blockchain.upgradable_read(),
                                    Block::Macro(batch_set.block),
                                    &batch_set.history,
                                )
                            })
                            .await
                            .expect("blockchain.push_history_sync() should not panic");

                            if let Err(e) = &result {
                                log::warn!("Failed to push epoch: {:?}", e);
                            }
                            result.into()
                        }
                        .boxed();

                        self.job_queue
                            .push_back(Job::PushBatchSet(cluster.id, hash, future));
                    }
                    Some(Err(_)) | None => {
                        // Cluster finished or errored, evict it.
                        let cluster = self.active_cluster.take().unwrap();

                        let result = match result {
                            Some(Err(e)) => e,
                            None => SyncClusterResult::NoMoreEpochs,
                            _ => unreachable!(),
                        };
                        self.job_queue
                            .push_back(Job::FinishCluster(cluster, result));

                        self.waker.wake();
                        break;
                    }
                }
            }
        }
    }

    fn poll_job_queue(&mut self, cx: &mut Context<'_>) {
        while let Some(job) = self.job_queue.front_mut() {
            let result = match job {
                Job::PushBatchSet(_, _, future) => match future.poll_unpin(cx) {
                    Poll::Ready(result) => Some(result),
                    Poll::Pending => break,
                },
                Job::FinishCluster(_, _) => None,
            };

            let job = self.job_queue.pop_front().unwrap();

            match job {
                Job::PushBatchSet(cluster_id, ..) => {
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

            self.waker.wake();
        }
    }

    fn evict_jobs_by_cluster(&mut self, cluster_id: usize) -> Option<SyncCluster<TNetwork>> {
        while let Some(job) = self.job_queue.front() {
            let id = match job {
                Job::PushBatchSet(cluster_id, ..) => *cluster_id,
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

impl<TNetwork: Network> Stream for HistoryMacroSync<TNetwork> {
    type Item = MacroSyncReturn<TNetwork::PeerId>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.waker.store_waker(cx);

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

#[cfg(test)]
mod tests {
    use std::{sync::Arc, task::Poll};

    use futures::{Stream, StreamExt};
    use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
    use nimiq_blockchain_interface::AbstractBlockchain;
    use nimiq_blockchain_proxy::BlockchainProxy;
    use nimiq_database::mdbx::MdbxDatabase;
    use nimiq_network_interface::{network::Network, request::request_handler};
    use nimiq_network_mock::{MockHub, MockNetwork};
    use nimiq_primitives::{networks::NetworkId, policy::Policy};
    use nimiq_test_log::test;
    use nimiq_test_utils::blockchain::{produce_macro_blocks_with_txns, signing_key, voting_key};
    use nimiq_utils::{spawn::spawn, time::OffsetTime};
    use parking_lot::RwLock;

    use crate::{
        messages::{RequestBatchSet, RequestHistoryChunk, RequestMacroChain},
        sync::{history::HistoryMacroSync, syncer::MacroSyncReturn},
    };

    fn blockchain() -> Arc<RwLock<Blockchain>> {
        let time = Arc::new(OffsetTime::new());
        let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
        Arc::new(RwLock::new(
            Blockchain::new(
                env,
                BlockchainConfig::default(),
                NetworkId::UnitAlbatross,
                time,
            )
            .unwrap(),
        ))
    }

    fn copy_chain(from: &RwLock<Blockchain>, to: &RwLock<Blockchain>) {
        copy_chain_with_limit(from, to, usize::MAX);
    }

    fn copy_chain_with_limit(
        from: &RwLock<Blockchain>,
        to: &RwLock<Blockchain>,
        num_blocks_max: usize,
    ) {
        let chain_info =
            from.read()
                .chain_store
                .get_chain_info(&to.read().head_hash(), false, None);
        let mut block_hash = match chain_info {
            Ok(chain_info) if chain_info.on_main_chain => chain_info.main_chain_successor,
            _ => panic!("Chains have diverged"),
        };

        let mut num_blocks = 0;
        while let Some(hash) = block_hash {
            let chain_info = from
                .read()
                .chain_store
                .get_chain_info(&hash, true, None)
                .unwrap();
            assert!(chain_info.on_main_chain);

            Blockchain::push(to.upgradable_read(), chain_info.head).expect("Failed to push block");
            block_hash = chain_info.main_chain_successor;

            num_blocks += 1;
            if num_blocks >= num_blocks_max {
                return;
            }
        }

        assert_eq!(from.read().head(), to.read().head());
    }

    fn spawn_request_handlers<TNetwork: Network>(
        network: &Arc<TNetwork>,
        blockchain: &Arc<RwLock<Blockchain>>,
    ) {
        spawn(request_handler(
            network,
            network.receive_requests::<RequestMacroChain>(),
            &BlockchainProxy::from(blockchain),
        ));
        spawn(request_handler(
            network,
            network.receive_requests::<RequestBatchSet>(),
            blockchain,
        ));
        spawn(request_handler(
            network,
            network.receive_requests::<RequestHistoryChunk>(),
            blockchain,
        ));
    }

    #[test(tokio::test)]
    async fn it_terminates_if_there_is_nothing_to_sync() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        let chain = blockchain();
        let mut sync = HistoryMacroSync::<MockNetwork>::new(
            Arc::clone(&chain),
            Arc::clone(&net1),
            net1.subscribe_events(),
        );

        spawn_request_handlers(&net2, &chain);
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain.read().block_number(), Policy::genesis_block_number());
            }
            res => panic!("Unexpected HistorySyncReturn: {res:?}"),
        }
    }

    #[test(tokio::test)]
    async fn it_can_sync_a_single_finalized_epoch() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        let chain1 = blockchain();
        let chain2 = blockchain();

        let producer = BlockProducer::new(signing_key(), voting_key());
        produce_macro_blocks_with_txns(
            &producer,
            &chain2,
            Policy::batches_per_epoch() as usize,
            1,
            0,
        );
        assert_eq!(
            chain2.read().block_number(),
            Policy::blocks_per_epoch() + Policy::genesis_block_number()
        );

        let mut sync = HistoryMacroSync::<MockNetwork>::new(
            Arc::clone(&chain1),
            Arc::clone(&net1),
            net1.subscribe_events(),
        );

        spawn_request_handlers(&net2, &chain2);
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().head(), chain2.read().head());
            }
            res => panic!("Unexpected HistorySyncReturn: {res:?}"),
        }
    }

    #[test(tokio::test)]
    async fn it_can_sync_multiple_finalized_epochs() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        let chain1 = blockchain();
        let chain2 = blockchain();

        let num_epochs = 2;
        let producer = BlockProducer::new(signing_key(), voting_key());
        produce_macro_blocks_with_txns(
            &producer,
            &chain2,
            num_epochs * Policy::batches_per_epoch() as usize,
            1,
            0,
        );
        assert_eq!(
            chain2.read().block_number(),
            num_epochs as u32 * Policy::blocks_per_epoch() + Policy::genesis_block_number()
        );

        let mut sync = HistoryMacroSync::<MockNetwork>::new(
            Arc::clone(&chain1),
            Arc::clone(&net1),
            net1.subscribe_events(),
        );

        spawn_request_handlers(&net2, &chain2);
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().head(), chain2.read().head());
            }
            res => panic!("Unexpected HistorySyncReturn: {res:?}"),
        }
    }

    #[test(tokio::test)]
    async fn it_can_sync_a_single_batch() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        let chain1 = blockchain();
        let chain2 = blockchain();

        let producer = BlockProducer::new(signing_key(), voting_key());
        produce_macro_blocks_with_txns(&producer, &chain2, 1, 1, 0);
        assert_eq!(
            chain2.read().block_number(),
            Policy::blocks_per_batch() + Policy::genesis_block_number()
        );

        let mut sync = HistoryMacroSync::<MockNetwork>::new(
            Arc::clone(&chain1),
            Arc::clone(&net1),
            net1.subscribe_events(),
        );

        spawn_request_handlers(&net2, &chain2);
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().head(), chain2.read().head());
            }
            res => panic!("Unexpected HistorySyncReturn: {res:?}"),
        }
    }

    #[test(tokio::test)]
    async fn it_can_sync_multiple_batches() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        let chain1 = blockchain();
        let chain2 = blockchain();

        let num_batches = (Policy::batches_per_epoch() - 1) as usize;
        let producer = BlockProducer::new(signing_key(), voting_key());
        produce_macro_blocks_with_txns(&producer, &chain2, num_batches, 1, 0);
        assert_eq!(
            chain2.read().block_number(),
            num_batches as u32 * Policy::blocks_per_batch() + Policy::genesis_block_number()
        );

        let mut sync = HistoryMacroSync::<MockNetwork>::new(
            Arc::clone(&chain1),
            Arc::clone(&net1),
            net1.subscribe_events(),
        );

        spawn_request_handlers(&net2, &chain2);
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().head(), chain2.read().head());
            }
            _ => panic!("Unexpected HistorySyncReturn"),
        }
    }

    #[test(tokio::test)]
    async fn it_can_sync_a_partial_epoch() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());

        let chain1 = blockchain();
        let chain2 = blockchain();

        let num_batches = 2usize;
        let producer = BlockProducer::new(signing_key(), voting_key());
        produce_macro_blocks_with_txns(&producer, &chain2, num_batches, 50, 0);
        assert_eq!(
            chain2.read().block_number(),
            num_batches as u32 * Policy::blocks_per_batch() + Policy::genesis_block_number()
        );

        let num_blocks = Policy::blocks_per_batch() + Policy::blocks_per_batch() / 2;
        copy_chain_with_limit(&chain2, &chain1, num_blocks as usize);
        assert_eq!(
            chain1.read().block_number(),
            num_blocks + Policy::genesis_block_number()
        );

        let mut sync = HistoryMacroSync::<MockNetwork>::new(
            Arc::clone(&chain1),
            Arc::clone(&net1),
            net1.subscribe_events(),
        );

        spawn_request_handlers(&net2, &chain2);
        net1.dial_mock(&net2);

        match sync.next().await {
            Some(MacroSyncReturn::Good(_)) => {
                assert_eq!(chain1.read().head(), chain2.read().head());
            }
            _ => panic!("Unexpected HistorySyncReturn"),
        }
    }

    #[test(tokio::test)]
    async fn it_can_sync_consecutive_batches_from_different_peers() {
        let mut hub = MockHub::default();
        let net1 = Arc::new(hub.new_network());
        let net2 = Arc::new(hub.new_network());
        let net3 = Arc::new(hub.new_network());
        let net4 = Arc::new(hub.new_network());

        let chain1 = blockchain();
        let chain2 = blockchain();
        let chain3 = blockchain();
        let chain4 = blockchain();

        let producer = BlockProducer::new(signing_key(), voting_key());
        produce_macro_blocks_with_txns(&producer, &chain2, 1, 1, 0);
        assert_eq!(
            chain2.read().block_number(),
            Policy::blocks_per_batch() + Policy::genesis_block_number()
        );

        copy_chain(&chain2, &chain3);
        produce_macro_blocks_with_txns(&producer, &chain3, 1, 1, 0);
        assert_eq!(
            chain3.read().block_number(),
            2 * Policy::blocks_per_batch() + Policy::genesis_block_number()
        );

        copy_chain(&chain3, &chain4);
        produce_macro_blocks_with_txns(&producer, &chain4, 1, 1, 0);
        assert_eq!(
            chain4.read().block_number(),
            3 * Policy::blocks_per_batch() + Policy::genesis_block_number()
        );

        let mut sync = HistoryMacroSync::<MockNetwork>::new(
            Arc::clone(&chain1),
            Arc::clone(&net1),
            net1.subscribe_events(),
        );

        spawn_request_handlers(&net2, &chain2);
        spawn_request_handlers(&net3, &chain3);
        spawn_request_handlers(&net4, &chain4);

        net1.dial_mock(&net2);
        net1.dial_mock(&net3);
        net1.dial_mock(&net4);

        match sync.next().await {
            Some(MacroSyncReturn::Good(peer_id)) if peer_id == net4.peer_id() => {}
            res => panic!("Unexpected HistorySyncReturn: {res:?}"),
        }
        match sync.next().await {
            Some(MacroSyncReturn::Outdated(peer_id)) if peer_id == net3.peer_id() => {}
            res => panic!("Unexpected HistorySyncReturn: {res:?}"),
        }
        match sync.next().await {
            Some(MacroSyncReturn::Outdated(peer_id)) if peer_id == net2.peer_id() => {}
            res => panic!("Unexpected HistorySyncReturn: {res:?}"),
        }

        assert_eq!(chain1.read().head(), chain4.read().head());
    }

    struct DisconnectDuringSyncStream {
        pub sync: HistoryMacroSync<MockNetwork>,
        pub net_sync: Arc<MockNetwork>,
        pub net_disconnect: Arc<MockNetwork>,
        pub chain_sync: Arc<RwLock<Blockchain>>,
        pub chain_up2date: Arc<RwLock<Blockchain>>,
        pub current_i: u32,
        pub disconnect_at: u32,
        pub reconnect_at: Option<u32>,
    }

    impl DisconnectDuringSyncStream {
        pub fn new(disconnect_at: u32, reconnect_at: Option<u32>) -> DisconnectDuringSyncStream {
            let mut hub = MockHub::default();
            let net_sync = Arc::new(hub.new_network());
            let net_up2date = Arc::new(hub.new_network());
            let net_disconnect = Arc::new(hub.new_network());

            let chain_sync = blockchain();
            let chain_up2date = blockchain();
            let chain_disconnect = blockchain();

            let num_epochs = 2;
            let producer = BlockProducer::new(signing_key(), voting_key());
            produce_macro_blocks_with_txns(
                &producer,
                &chain_up2date,
                (num_epochs * Policy::batches_per_epoch() - 1) as usize,
                1,
                0,
            );
            copy_chain(&chain_up2date, &chain_disconnect);

            let sync = HistoryMacroSync::<MockNetwork>::new(
                Arc::clone(&chain_sync),
                Arc::clone(&net_sync),
                net_sync.subscribe_events(),
            );

            spawn_request_handlers(&net_sync, &chain_sync);
            spawn_request_handlers(&net_up2date, &chain_up2date);
            spawn_request_handlers(&net_disconnect, &chain_disconnect);

            net_sync.dial_mock(&net_up2date);
            net_sync.dial_mock(&net_disconnect);

            DisconnectDuringSyncStream {
                sync,
                net_sync,
                net_disconnect,
                chain_sync,
                chain_up2date,
                current_i: 0,
                disconnect_at,
                reconnect_at,
            }
        }
    }

    impl Stream for DisconnectDuringSyncStream {
        type Item = bool;

        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> Poll<Option<Self::Item>> {
            match self.sync.poll_next_unpin(cx) {
                Poll::Ready(Some(MacroSyncReturn::Good(_))) => {
                    assert_eq!(
                        self.chain_sync.read().head(),
                        self.chain_up2date.read().head()
                    );
                    return Poll::Ready(Some(true));
                }
                Poll::Pending => {
                    if self.current_i == self.disconnect_at {
                        self.net_disconnect.disconnect();
                    }
                    if let Some(rec_at) = self.reconnect_at {
                        if self.current_i == rec_at {
                            self.net_sync.dial_mock(&self.net_disconnect);
                        }
                    }
                }
                _ => return Poll::Ready(None),
            }
            self.current_i += 1;
            Poll::Pending
        }
    }

    #[test(tokio::test)]
    async fn it_can_disconnect_peer_during_sync() {
        // The to-be-synced peer is connected with an up-to-date one the whole time. An additional up-to-date peer is disconnected (and reconnected).

        // Disconnect at the beginning, don't reconnect
        let mut res = DisconnectDuringSyncStream::new(2, None);
        assert!(res.next().await.unwrap());

        // Disconnect after 1/3 of the stream polls, reconnect at 2/3.
        let mut res =
            DisconnectDuringSyncStream::new(res.current_i / 3, Some(res.current_i / 3 * 2));
        assert!(res.next().await.unwrap());
    }
}
