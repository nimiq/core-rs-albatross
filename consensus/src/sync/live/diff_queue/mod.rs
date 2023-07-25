use std::{
    collections::HashSet,
    future::Future,
    ops::RangeTo,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    future::BoxFuture,
    stream::{FuturesOrdered, FuturesUnordered},
    Stream, StreamExt, TryStreamExt,
};
use nimiq_block::Block;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::Network,
    request::{RequestCommon, RequestMarker},
};
use nimiq_primitives::{key_nibbles::KeyNibbles, trie::trie_diff::TrieDiff};
use nimiq_serde::{Deserialize, Serialize};
use parking_lot::RwLock;

use self::diff_request_component::DiffRequestComponent;
use super::block_queue::{BlockAndId, BlockQueue, QueuedBlock};
use crate::sync::{
    live::{block_queue::live_sync::PushOpResult, queue::LiveSyncQueue},
    peer_list::PeerList,
    syncer::LiveSyncEvent,
};

pub mod diff_request_component;

/// The max number of partial trie diffs requests per peer.
pub const MAX_REQUEST_RESPONSE_PARTIAL_DIFFS: u32 = 100;

/// The request of a trie diff.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestPartialDiff {
    pub block_hash: Blake2bHash,
    pub range: RangeTo<KeyNibbles>,
}

/// The response for trie diff requests.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum ResponsePartialDiff {
    PartialDiff(TrieDiff),
    UnknownBlockHash,
    IncompleteState,
}

impl RequestCommon for RequestPartialDiff {
    type Kind = RequestMarker;
    const TYPE_ID: u16 = 218;
    type Response = ResponsePartialDiff;

    const MAX_REQUESTS: u32 = MAX_REQUEST_RESPONSE_PARTIAL_DIFFS;
}

pub enum QueuedDiff<N: Network> {
    Head(BlockAndId<N>, TrieDiff),
    Buffered(Vec<(BlockAndId<N>, TrieDiff)>),
    Missing(Vec<(Block, TrieDiff)>),
    TooFarAhead(N::PeerId),
    TooFarBehind(N::PeerId),
    PeerIncompleteState(N::PeerId),
}

async fn augment_block<N, F, R>(block: QueuedBlock<N>, mut get_diff: F) -> Result<QueuedDiff<N>, ()>
where
    N: Network,
    F: FnMut(&BlockAndId<N>) -> R,
    R: Future<Output = Result<TrieDiff, ()>>,
{
    async fn get_multiple_diffs<N, F, R>(
        blocks: &[BlockAndId<N>],
        mut get_diff: F,
    ) -> Result<Vec<TrieDiff>, ()>
    where
        N: Network,
        F: FnMut(&BlockAndId<N>) -> R,
        R: Future<Output = Result<TrieDiff, ()>>,
    {
        // This is just a fancy way to collect all the diffs simultaneously.
        let diffs: FuturesUnordered<_> = blocks
            .iter()
            .enumerate()
            .map(|(i, block)| {
                // Get each diff.
                let diff = get_diff(block);
                async move {
                    // Annotate it with its index.
                    diff.await.map(|d| (i, d))
                }
            })
            .collect();

        // Collect all diffs, returning an error if any failed.
        let mut diffs = diffs.try_collect::<Vec<_>>().await?;

        // Sort the diffs by index again.
        diffs.sort_unstable_by_key(|&(i, _)| i);
        assert_eq!(blocks.len(), diffs.len());

        // Strip index before returning.
        Ok(diffs.into_iter().map(|(_, diff)| diff).collect())
    }

    Ok(match block {
        QueuedBlock::Head(block) => {
            let diff = get_diff(&block).await?;
            QueuedDiff::Head(block, diff)
        }
        QueuedBlock::Buffered(blocks) => {
            let diffs = get_multiple_diffs::<N, F, R>(&blocks[..], get_diff).await?;
            QueuedDiff::Buffered(blocks.into_iter().zip(diffs).collect())
        }
        QueuedBlock::Missing(blocks) => {
            let blocks = blocks
                .into_iter()
                .map(|block| (block, Option::<N::PubsubId>::None))
                .collect::<Vec<_>>();
            let diffs = get_multiple_diffs::<N, F, R>(&blocks[..], get_diff).await?;
            QueuedDiff::Missing(
                blocks
                    .into_iter()
                    .map(|(block, _)| block)
                    .zip(diffs)
                    .collect(),
            )
        }
        QueuedBlock::TooFarAhead(_, peer_id) => QueuedDiff::TooFarAhead(peer_id),
        QueuedBlock::TooFarBehind(_, peer_id) => QueuedDiff::TooFarBehind(peer_id),
    })
}

pub struct DiffQueue<N: Network> {
    /// The BlockQueue component.
    block_queue: BlockQueue<N>,

    /// The chunk request component.
    /// We use it to request chunks from up-to-date peers
    diff_request_component: DiffRequestComponent<N>,

    /// The pending TreeDiff requests to peers.
    diffs: FuturesOrdered<BoxFuture<'static, Result<QueuedDiff<N>, ()>>>,
}

impl<N: Network> DiffQueue<N> {
    pub fn with_block_queue(network: Arc<N>, block_queue: BlockQueue<N>) -> Self {
        let diff_request_component =
            DiffRequestComponent::new(Arc::clone(&network), block_queue.peer_list());
        Self {
            block_queue,
            diff_request_component,
            diffs: FuturesOrdered::new(),
        }
    }

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

    /// Adds an additional block stream by replacing the current block stream with a `select` of both streams.
    pub(crate) fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = (Block, N::PeerId, Option<N::PubsubId>)> + Send + 'static,
    {
        self.block_queue.add_block_stream(block_stream)
    }

    pub(crate) fn num_buffered_blocks(&self) -> usize {
        self.block_queue.num_buffered_blocks()
    }
}

impl<N: Network> Stream for DiffQueue<N> {
    type Item = QueuedDiff<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let mut block_queue_done = false;

        // Receive blocks from BlockQueue.
        loop {
            match self.block_queue.poll_next_unpin(cx) {
                Poll::Ready(Some(block)) => {
                    let get_diff = self.diff_request_component.request_diff(..KeyNibbles::ROOT);
                    self.diffs
                        .push_back(Box::pin(augment_block(block, get_diff)));
                }
                Poll::Ready(None) => {
                    block_queue_done = true;
                    break;
                }
                Poll::Pending => break,
            }
        }

        // Check for blocks augmented with diffs.
        loop {
            match self.diffs.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(diff))) => return Poll::Ready(Some(diff)),
                Poll::Ready(Some(Err(()))) => {
                    error!("couldn't fetch diff");
                    continue;
                }
                Poll::Ready(None) if block_queue_done => return Poll::Ready(None),
                Poll::Ready(None) => break,
                Poll::Pending => break,
            }
        }

        Poll::Pending
    }
}
