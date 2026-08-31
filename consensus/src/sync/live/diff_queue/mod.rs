//! Provides a queuing component that augments blocks with state trie diffs during live sync.

use std::{
    collections::{HashMap, HashSet},
    future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{future::BoxFuture, Stream, StreamExt};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::Network,
    request::{RequestCommon, RequestMarker},
};
use nimiq_primitives::trie::trie_diff::TrieDiff;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::stream::FuturesOrdered;
use parking_lot::RwLock;

use self::diff_request_component::{DiffRequestComponent, DiffRequestError};
use super::block_queue::{BlockAndSource, BlockQueue, BlockSource, QueuedBlock};
use crate::{
    consensus::ResolveBlockRequest,
    sync::{
        live::{block_queue::live_sync::PushOpResult, queue::LiveSyncQueue},
        peer_list::PeerList,
        sync_interface::LiveSyncEvent,
    },
};

pub mod diff_request_component;

/// The max number of partial trie diffs requests per peer.
pub const MAX_REQUEST_RESPONSE_TRIE_DIFFS: u32 = 100;

/// The request of a trie diff.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestTrieDiff {
    pub block_hash: Blake2bHash,
}

/// The response for trie diff requests.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ResponseTrieDiff {
    PartialDiff(TrieDiff),
    UnknownBlockHash,
    IncompleteState,
}

impl RequestCommon for RequestTrieDiff {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 218;
    type Response = ResponseTrieDiff;

    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_TRIE_DIFFS;
}

/// Represents a block or set of blocks classified by how they should be processed.
pub enum QueuedDiff<N: Network> {
    Head(BlockAndSource<N>, Option<TrieDiff>),
    Buffered(Vec<(BlockAndSource<N>, Option<TrieDiff>)>),
    Missing(Vec<(BlockAndSource<N>, Option<TrieDiff>)>),
    TooFarAhead(N::PeerId),
    TooFarBehind(N::PeerId),
    PeerIncompleteState(N::PeerId),
}

impl<N: Network> QueuedDiff<N> {
    fn from_block_no_diff(block: QueuedBlock<N>) -> Self {
        match block {
            QueuedBlock::Head(block) => QueuedDiff::Head(block, None),
            QueuedBlock::Buffered(blocks) => {
                QueuedDiff::Buffered(blocks.into_iter().map(|block| (block, None)).collect())
            }
            QueuedBlock::Missing(blocks) => {
                QueuedDiff::Missing(blocks.into_iter().map(|block| (block, None)).collect())
            }
            QueuedBlock::TooFarAhead(peer_id) => QueuedDiff::TooFarAhead(peer_id),
            QueuedBlock::TooFarBehind(peer_id) => QueuedDiff::TooFarBehind(peer_id),
        }
    }
}

struct AugmentedDiff<N: Network> {
    queued_diff: Option<QueuedDiff<N>>,
    dropped_blocks: HashMap<Blake2bHash, BlockSource<N>>,
}

/// Fetches a batch concurrently while consuming its results in block order.
///
/// Requests after the first failing block are speculative: they may already have started or even
/// completed before the ordered failure is observed. Only the successful prefix is retained.
async fn get_multiple_diffs<N: Network>(
    blocks: Vec<BlockAndSource<N>>,
    mut get_diff: impl FnMut(
        &BlockAndSource<N>,
    ) -> BoxFuture<'static, Result<TrieDiff, DiffRequestError>>,
) -> (
    Vec<(BlockAndSource<N>, Option<TrieDiff>)>,
    HashMap<Blake2bHash, BlockSource<N>>,
) {
    let mut blocks_with_diffs = Vec::with_capacity(blocks.len());
    // `FuturesOrdered` polls all requests concurrently, but only yields them in insertion order.
    // Indexing the results makes the prefix invariant explicit and avoids sorting after retrieval.
    let mut diff_futures: FuturesOrdered<_> = blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let diff = get_diff(block);
            async move { (index, diff.await) }
        })
        .collect();
    let mut remaining_blocks = blocks.into_iter();

    while let Some((index, result)) = diff_futures.next().await {
        let block = remaining_blocks
            .next()
            .expect("one diff future is created per block");
        debug_assert_eq!(index, blocks_with_diffs.len());

        match result {
            Ok(diff) => blocks_with_diffs.push((block, Some(diff))),
            Err(DiffRequestError::MaxTriesExceeded) => {
                let dropped_blocks = std::iter::once(block)
                    .chain(remaining_blocks)
                    .map(|(block, block_source)| (block.hash(), block_source))
                    .collect();
                // Returning drops the speculative suffix futures, stopping their local retry loops
                // and releasing their semaphore permits. Requests already handed to the network
                // cannot be withdrawn and may still complete; their responses are ignored.
                return (blocks_with_diffs, dropped_blocks);
            }
        }
    }

    (blocks_with_diffs, HashMap::default())
}

async fn augment_block<N: Network>(
    block: QueuedBlock<N>,
    mut get_diff: impl FnMut(
        &BlockAndSource<N>,
    ) -> BoxFuture<'static, Result<TrieDiff, DiffRequestError>>,
) -> AugmentedDiff<N> {
    match block {
        QueuedBlock::Head(block) => match get_diff(&block).await {
            Ok(diff) => AugmentedDiff {
                queued_diff: Some(QueuedDiff::Head(block, Some(diff))),
                dropped_blocks: HashMap::default(),
            },
            Err(DiffRequestError::MaxTriesExceeded) => {
                let (block, block_source) = block;
                AugmentedDiff {
                    queued_diff: None,
                    dropped_blocks: HashMap::from([(block.hash(), block_source)]),
                }
            }
        },
        QueuedBlock::Buffered(blocks) => {
            let (blocks_with_diffs, dropped_blocks) = get_multiple_diffs(blocks, get_diff).await;
            AugmentedDiff {
                queued_diff: (!blocks_with_diffs.is_empty())
                    .then_some(QueuedDiff::Buffered(blocks_with_diffs)),
                dropped_blocks,
            }
        }
        QueuedBlock::Missing(blocks) => {
            let (blocks_with_diffs, dropped_blocks) = get_multiple_diffs(blocks, get_diff).await;
            AugmentedDiff {
                queued_diff: (!blocks_with_diffs.is_empty())
                    .then_some(QueuedDiff::Missing(blocks_with_diffs)),
                dropped_blocks,
            }
        }
        QueuedBlock::TooFarAhead(peer_id) => AugmentedDiff {
            queued_diff: Some(QueuedDiff::TooFarAhead(peer_id)),
            dropped_blocks: HashMap::default(),
        },
        QueuedBlock::TooFarBehind(peer_id) => AugmentedDiff {
            queued_diff: Some(QueuedDiff::TooFarBehind(peer_id)),
            dropped_blocks: HashMap::default(),
        },
    }
}

pub struct DiffQueue<N: Network> {
    /// Reference to the network used to close validation of dropped gossip messages.
    network: Arc<N>,

    /// The BlockQueue component.
    block_queue: BlockQueue<N>,

    /// The chunk request component.
    /// We use it to request chunks from up-to-date peers
    diff_request_component: DiffRequestComponent<N>,

    /// The pending TreeDiff requests to peers.
    diffs: FuturesOrdered<BoxFuture<'static, AugmentedDiff<N>>>,

    /// Flag indicating if diffs should be requested.
    diff_needed: bool,
}

impl<N: Network> DiffQueue<N> {
    /// Creates a new DiffQueue using an existing BlockQueue.
    pub fn with_block_queue(network: Arc<N>, block_queue: BlockQueue<N>) -> Self {
        let diff_request_component =
            DiffRequestComponent::new(Arc::clone(&network), block_queue.peer_list());
        Self {
            network,
            block_queue,
            diff_request_component,
            diffs: FuturesOrdered::new(),
            diff_needed: true,
        }
    }

    /// Removes blocks marked as invalid from the BlockQueue.
    pub(crate) fn remove_invalid_blocks(&mut self, invalid_blocks: &mut HashSet<Blake2bHash>) {
        // We remove invalid blocks from the block queue.
        self.block_queue.remove_invalid_blocks(invalid_blocks);
    }

    pub(crate) fn process_push_result(
        &mut self,
        item: PushOpResult<N>,
    ) -> Option<LiveSyncEvent<N::PeerId>> {
        self.block_queue.process_push_result(item)
    }

    pub(crate) fn peers(&self) -> Vec<N::PeerId> {
        self.block_queue.peers()
    }

    pub(crate) fn peer_list(&self) -> Arc<RwLock<PeerList<N>>> {
        self.block_queue.peer_list()
    }

    pub(crate) fn num_peers(&self) -> usize {
        self.block_queue.num_peers()
    }

    pub(crate) fn add_peer(&self, peer_id: N::PeerId) {
        self.block_queue.add_peer(peer_id)
    }

    /// Adds a block stream by replacing the current block stream with a `select` of both streams.
    pub(crate) fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = BlockAndSource<N>> + Send + 'static,
    {
        self.block_queue.add_block_stream(block_stream)
    }

    pub(crate) fn num_buffered_heights(&self) -> usize {
        self.block_queue.num_buffered_heights()
    }

    /// Sets whether diffs should be fetched for incoming blocks.
    pub(crate) fn set_diff_needed(&mut self, diff_needed: bool) {
        self.diff_needed = diff_needed;
    }

    pub(crate) fn resolve_block(&mut self, request: ResolveBlockRequest<N>) {
        self.block_queue.resolve_block(request)
    }

    pub(crate) fn acceptance_window_size(&self) -> u32 {
        self.block_queue.acceptance_window_size()
    }

    /// Ignores announced blocks whose diff requests failed and clears their pending markers.
    /// A request failure is not proof of invalidity: `remove_invalid_blocks` would also reject
    /// buffered descendants. `Ignore` closes gossipsub validation without penalizing the source,
    /// while clearing `blocks_pending_push` allows a later announcement to enqueue the block again.
    fn clear_pending_diff_requests(
        &mut self,
        dropped_blocks: HashMap<Blake2bHash, BlockSource<N>>,
    ) {
        for (block_hash, block_source) in dropped_blocks {
            block_source.ignore_block(&self.network);
            self.block_queue.on_block_processed(&block_hash);
        }
    }
}

impl<N: Network> Stream for DiffQueue<N> {
    type Item = QueuedDiff<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Receive blocks from BlockQueue.
        while let Poll::Ready(block) = self.block_queue.poll_next_unpin(cx) {
            match (block, self.diff_needed) {
                (Some(block), true) => {
                    let get_diff = self.diff_request_component.request_diff();
                    self.diffs
                        .push_back(Box::pin(augment_block(block, get_diff)));
                }
                (Some(block), false) => {
                    self.diffs.push_back(Box::pin(future::ready(AugmentedDiff {
                        queued_diff: Some(QueuedDiff::from_block_no_diff(block)),
                        dropped_blocks: HashMap::default(),
                    })));
                }
                // The block queue only ends when something bad happens.
                // Thus we immediately quit and do not wait for any pending diffs.
                (None, ..) => return Poll::Ready(None),
            }
        }

        // Check for blocks augmented with diffs.
        while let Poll::Ready(Some(diff)) = self.diffs.poll_next_unpin(cx) {
            if !diff.dropped_blocks.is_empty() {
                debug!(
                    dropped_block_hashes = ?diff.dropped_blocks.keys(),
                    "dropping blocks after diff request failed"
                );
                self.clear_pending_diff_requests(diff.dropped_blocks);
            }

            if let Some(queued_diff) = diff.queued_diff {
                return Poll::Ready(Some(queued_diff));
            }
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        future::{pending, ready},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use futures::{join, StreamExt};
    use nimiq_blockchain_proxy::BlockchainProxy;
    use nimiq_network_interface::network::{MsgAcceptance, Network, PubsubId};
    use nimiq_network_mock::{MockHub, MockId, MockNetwork, MockPeerId};
    use nimiq_primitives::trie::trie_diff::TrieDiff;
    use nimiq_test_log::test;
    use nimiq_test_utils::block_production::TemporaryBlockProducer;
    use tokio::{
        sync::{mpsc, Notify},
        time::timeout,
    };
    use tokio_stream::wrappers::ReceiverStream;

    use super::{
        augment_block, diff_request_component::DiffRequestError, AugmentedDiff, DiffQueue,
        QueuedDiff, RequestTrieDiff, ResponseTrieDiff,
    };
    use crate::sync::live::{
        block_queue::{BlockQueue, BlockSource, QueuedBlock},
        queue::QueueConfig,
    };

    #[test(tokio::test)]
    async fn failed_head_diff_request_ignores_message_and_allows_reannouncement() {
        let mut hub = MockHub::new();
        let local = TemporaryBlockProducer::new();
        let local_blockchain = Arc::clone(&local.blockchain);
        let network = Arc::new(hub.new_network());
        let (block_tx, block_rx) = mpsc::channel(8);

        let block_queue = BlockQueue::with_gossipsub_block_stream(
            BlockchainProxy::from(&local_blockchain),
            Arc::clone(&network),
            ReceiverStream::new(block_rx).boxed(),
            QueueConfig::default(),
        );
        let mut diff_queue = DiffQueue::with_block_queue(Arc::clone(&network), block_queue);

        let source = TemporaryBlockProducer::new();
        let source_network = Arc::new(hub.new_network());
        let other_network = Arc::new(hub.new_network());
        let mut source_requests = source_network.receive_requests::<RequestTrieDiff>();
        let mut other_requests = other_network.receive_requests::<RequestTrieDiff>();

        network.dial_mock(&source_network);
        network.dial_mock(&other_network);
        diff_queue.add_peer(source_network.get_local_peer_id());
        diff_queue.add_peer(other_network.get_local_peer_id());

        let block = source.next_block(vec![], false);
        let source_id = MockId::new(source_network.get_local_peer_id());
        block_tx
            .send((block.clone(), source_id.clone()))
            .await
            .unwrap();

        let (_, first_attempt) = join!(
            async {
                let (_, request_id, _) = source_requests.next().await.unwrap();
                source_network
                    .respond::<RequestTrieDiff>(request_id, ResponseTrieDiff::IncompleteState)
                    .await
                    .unwrap();

                let (_, request_id, _) = other_requests.next().await.unwrap();
                other_network
                    .respond::<RequestTrieDiff>(request_id, ResponseTrieDiff::UnknownBlockHash)
                    .await
                    .unwrap();
            },
            async { timeout(Duration::from_millis(100), diff_queue.next()).await },
        );
        assert!(
            first_attempt.is_err(),
            "diff request failure should not emit an augmented block"
        );
        let validation_results = network.take_validation_results();
        assert!(
            matches!(
                validation_results.as_slice(),
                [(_, pubsub_id, MsgAcceptance::Ignore)]
                    if pubsub_id.propagation_source() == source_id.propagation_source()
            ),
            "the dropped block announcement should be ignored without penalizing its source"
        );

        block_tx.send((block, source_id)).await.unwrap();

        let (retry_request, retry_attempt) = join!(
            async { timeout(Duration::from_millis(100), source_requests.next()).await },
            async { timeout(Duration::from_millis(100), diff_queue.next()).await },
        );
        assert!(
            matches!(retry_request, Ok(Some(_))),
            "the same head block should be accepted again after the previous diff request was dropped"
        );
        assert!(
            retry_attempt.is_err(),
            "reannouncing the head block should still wait on the next diff request"
        );
    }

    #[test(tokio::test)]
    async fn dropped_diff_does_not_block_follow_up_items() {
        let mut hub = MockHub::new();
        let local = TemporaryBlockProducer::new();
        let local_blockchain = Arc::clone(&local.blockchain);
        let network = Arc::new(hub.new_network());
        let (_block_tx, block_rx) = mpsc::channel(8);

        let block_queue = BlockQueue::with_gossipsub_block_stream(
            BlockchainProxy::from(&local_blockchain),
            Arc::clone(&network),
            ReceiverStream::new(block_rx).boxed(),
            QueueConfig::default(),
        );
        let mut diff_queue = DiffQueue::with_block_queue(Arc::clone(&network), block_queue);

        let peer_id = network.get_local_peer_id();
        diff_queue.diffs.push_back(Box::pin(ready(AugmentedDiff {
            queued_diff: None,
            dropped_blocks: HashMap::default(),
        })));
        diff_queue.diffs.push_back(Box::pin(ready(AugmentedDiff {
            queued_diff: Some(QueuedDiff::TooFarAhead(peer_id)),
            dropped_blocks: HashMap::default(),
        })));

        let next_item = timeout(Duration::from_millis(100), diff_queue.next()).await;
        assert!(
            matches!(next_item, Ok(Some(QueuedDiff::TooFarAhead(id))) if id == peer_id),
            "dropping a diff request failure must not block later queued items"
        );
    }

    #[test(tokio::test)]
    async fn buffered_diff_failure_keeps_prefix_and_reports_remaining_hashes() {
        let source = TemporaryBlockProducer::new();
        let first_block = source.next_block(vec![], false);
        let second_block = source.next_block(vec![], false);
        let third_block = source.next_block(vec![], false);
        let first_block_hash = first_block.hash();
        let second_block_hash = second_block.hash();
        let third_block_hash = third_block.hash();
        let failing_block_hash = second_block_hash.clone();

        let result = augment_block(
            QueuedBlock::<MockNetwork>::Buffered(vec![
                (
                    first_block,
                    BlockSource::<MockNetwork>::requested(MockPeerId(0)),
                ),
                (
                    second_block,
                    BlockSource::<MockNetwork>::requested(MockPeerId(0)),
                ),
                (
                    third_block,
                    BlockSource::<MockNetwork>::requested(MockPeerId(0)),
                ),
            ]),
            move |(block, _)| {
                let block_hash = block.hash();
                Box::pin(ready(if block_hash == failing_block_hash {
                    Err(DiffRequestError::MaxTriesExceeded)
                } else {
                    Ok(TrieDiff::default())
                }))
            },
        )
        .await;

        assert!(
            matches!(
                result,
                AugmentedDiff {
                    queued_diff: Some(QueuedDiff::Buffered(blocks_with_diffs)),
                    dropped_blocks,
                }
                    if blocks_with_diffs.len() == 1
                        && blocks_with_diffs[0].0.0.hash() == first_block_hash
                        && blocks_with_diffs[0].1.is_some()
                        && dropped_blocks.keys().cloned().collect::<HashSet<_>>()
                            == HashSet::from([second_block_hash, third_block_hash])
            ),
            "buffered diff failures should keep the successful prefix and drop the failed suffix"
        );
    }

    #[test(tokio::test)]
    async fn missing_diff_failure_keeps_prefix_and_reports_remaining_hashes() {
        let source = TemporaryBlockProducer::new();
        let first_block = source.next_block(vec![], false);
        let second_block = source.next_block(vec![], false);
        let third_block = source.next_block(vec![], false);
        let first_block_hash = first_block.hash();
        let second_block_hash = second_block.hash();
        let third_block_hash = third_block.hash();
        let failing_block_hash = second_block_hash.clone();

        let result = augment_block(
            QueuedBlock::<MockNetwork>::Missing(vec![
                (
                    first_block,
                    BlockSource::<MockNetwork>::requested(MockPeerId(0)),
                ),
                (
                    second_block,
                    BlockSource::<MockNetwork>::requested(MockPeerId(0)),
                ),
                (
                    third_block,
                    BlockSource::<MockNetwork>::requested(MockPeerId(0)),
                ),
            ]),
            move |(block, _)| {
                let block_hash = block.hash();
                Box::pin(ready(if block_hash == failing_block_hash {
                    Err(DiffRequestError::MaxTriesExceeded)
                } else {
                    Ok(TrieDiff::default())
                }))
            },
        )
        .await;

        assert!(
            matches!(
                result,
                AugmentedDiff {
                    queued_diff: Some(QueuedDiff::Missing(blocks_with_diffs)),
                    dropped_blocks,
                }
                    if blocks_with_diffs.len() == 1
                        && blocks_with_diffs[0].0.0.hash() == first_block_hash
                        && blocks_with_diffs[0].1.is_some()
                        && dropped_blocks.keys().cloned().collect::<HashSet<_>>()
                            == HashSet::from([second_block_hash, third_block_hash])
            ),
            "missing diff failures should keep the successful prefix and drop the failed suffix"
        );
    }

    #[test(tokio::test)]
    async fn missing_diff_requests_are_concurrent_but_stop_at_first_ordered_failure() {
        struct SetOnDrop(Arc<AtomicBool>);

        impl Drop for SetOnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let source = TemporaryBlockProducer::new();
        let first_block = source.next_block(vec![], false);
        let second_block = source.next_block(vec![], false);
        let third_block = source.next_block(vec![], false);
        let first_block_hash = first_block.hash();
        let second_block_hash = second_block.hash();
        let third_block_hash = third_block.hash();

        let started_requests = Arc::new(AtomicUsize::new(0));
        let started_requests_for_diff = Arc::clone(&started_requests);
        let release_first = Arc::new(Notify::new());
        let release_first_for_diff = Arc::clone(&release_first);
        let suffix_cancelled = Arc::new(AtomicBool::new(false));
        let suffix_cancelled_for_diff = Arc::clone(&suffix_cancelled);
        let first_request_hash = first_block_hash.clone();
        let second_request_hash = second_block_hash.clone();

        let request = augment_block(
            QueuedBlock::<MockNetwork>::Missing(vec![
                (
                    first_block,
                    BlockSource::<MockNetwork>::requested(MockPeerId(0)),
                ),
                (
                    second_block,
                    BlockSource::<MockNetwork>::requested(MockPeerId(0)),
                ),
                (
                    third_block,
                    BlockSource::<MockNetwork>::requested(MockPeerId(0)),
                ),
            ]),
            move |(block, _)| {
                let block_hash = block.hash();
                let started_requests = Arc::clone(&started_requests_for_diff);
                let release_first = Arc::clone(&release_first_for_diff);
                let suffix_cancelled = Arc::clone(&suffix_cancelled_for_diff);
                let first_request_hash = first_request_hash.clone();
                let second_request_hash = second_request_hash.clone();

                Box::pin(async move {
                    started_requests.fetch_add(1, Ordering::SeqCst);

                    if block_hash == first_request_hash {
                        // Hold the first result so that the ordered stream must poll the suffix.
                        release_first.notified().await;
                        Ok(TrieDiff::default())
                    } else if block_hash == second_request_hash {
                        Err(DiffRequestError::MaxTriesExceeded)
                    } else {
                        // This future must be dropped once the preceding error is consumed.
                        let _set_on_drop = SetOnDrop(suffix_cancelled);
                        pending().await
                    }
                })
            },
        );

        let observe_concurrency = async {
            timeout(Duration::from_secs(1), async {
                while started_requests.load(Ordering::SeqCst) != 3 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("all diff futures should be polled concurrently");
            release_first.notify_one();
        };

        let (result, ()) = join!(request, observe_concurrency);

        assert!(
            matches!(
                result,
                AugmentedDiff {
                    queued_diff: Some(QueuedDiff::Missing(blocks_with_diffs)),
                    dropped_blocks,
                }
                    if blocks_with_diffs.len() == 1
                        && blocks_with_diffs[0].0.0.hash() == first_block_hash
                        && blocks_with_diffs[0].1.is_some()
                        && dropped_blocks.keys().cloned().collect::<HashSet<_>>()
                            == HashSet::from([second_block_hash, third_block_hash])
            ),
            "ordered retrieval should retain the prefix and drop the failed suffix"
        );
        assert_eq!(started_requests.load(Ordering::SeqCst), 3);
        assert!(
            suffix_cancelled.load(Ordering::SeqCst),
            "the unconsumed local suffix future should be cancelled"
        );
    }
}
