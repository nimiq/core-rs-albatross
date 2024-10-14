use std::{
    collections::HashSet,
    future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{future::BoxFuture, Stream, StreamExt, TryStreamExt};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::Network,
    request::{RequestCommon, RequestMarker},
};
use nimiq_primitives::trie::trie_diff::TrieDiff;
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::stream::{FuturesOrdered, FuturesUnordered};
use parking_lot::RwLock;

use self::diff_request_component::DiffRequestComponent;
use super::block_queue::{BlockAndSource, BlockQueue, QueuedBlock};
use crate::{
    consensus::ResolveBlockRequest,
    sync::{
        live::{block_queue::live_sync::PushOpResult, queue::LiveSyncQueue},
        peer_list::PeerList,
        syncer::LiveSyncEvent,
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

async fn get_multiple_diffs<N: Network>(
    blocks: &[BlockAndSource<N>],
    mut get_diff: impl FnMut(&BlockAndSource<N>) -> BoxFuture<'static, Result<TrieDiff, ()>>,
) -> Result<Vec<TrieDiff>, ()> {
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

async fn augment_block<N: Network>(
    block: QueuedBlock<N>,
    mut get_diff: impl FnMut(&BlockAndSource<N>) -> BoxFuture<'static, Result<TrieDiff, ()>>,
) -> Result<QueuedDiff<N>, ()> {
    Ok(match block {
        QueuedBlock::Head(block) => {
            let diff = get_diff(&block).await?;
            QueuedDiff::Head(block, Some(diff))
        }
        QueuedBlock::Buffered(blocks) => {
            let diffs = get_multiple_diffs::<N>(&blocks[..], get_diff).await?;
            QueuedDiff::Buffered(
                blocks
                    .into_iter()
                    .zip(diffs.into_iter().map(Some))
                    .collect(),
            )
        }
        QueuedBlock::Missing(blocks) => {
            let diffs = get_multiple_diffs::<N>(&blocks[..], get_diff).await?;
            QueuedDiff::Missing(
                blocks
                    .into_iter()
                    .zip(diffs.into_iter().map(Some))
                    .collect(),
            )
        }
        QueuedBlock::TooFarAhead(peer_id) => QueuedDiff::TooFarAhead(peer_id),
        QueuedBlock::TooFarBehind(peer_id) => QueuedDiff::TooFarBehind(peer_id),
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

    /// Flag indicating if diffs should be requested.
    diff_needed: bool,
}

impl<N: Network> DiffQueue<N> {
    pub fn with_block_queue(network: Arc<N>, block_queue: BlockQueue<N>) -> Self {
        let diff_request_component =
            DiffRequestComponent::new(Arc::clone(&network), block_queue.peer_list());
        Self {
            block_queue,
            diff_request_component,
            diffs: FuturesOrdered::new(),
            diff_needed: true,
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

    /// Adds a block stream by replacing the current block stream with a `select` of both streams.
    pub(crate) fn add_block_stream<S>(&mut self, block_stream: S)
    where
        S: Stream<Item = BlockAndSource<N>> + Send + 'static,
    {
        self.block_queue.add_block_stream(block_stream)
    }

    pub(crate) fn num_buffered_blocks(&self) -> usize {
        self.block_queue.num_buffered_blocks()
    }

    pub(crate) fn set_diff_needed(&mut self, diff_needed: bool) {
        self.diff_needed = diff_needed;
    }

    pub(crate) fn resolve_block(&mut self, request: ResolveBlockRequest<N>) {
        self.block_queue.resolve_block(request)
    }

    pub(crate) fn acceptance_window_size(&self) -> u32 {
        self.block_queue.acceptance_window_size()
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
                    self.diffs.push_back(Box::pin(future::ready(Ok(
                        QueuedDiff::from_block_no_diff(block),
                    ))));
                }
                // The block queue only ends when something bad happens.
                // Thus we immediately quit and do not wait for any pending diffs.
                (None, ..) => return Poll::Ready(None),
            }
        }

        // Check for blocks augmented with diffs.
        while let Poll::Ready(Some(diff)) = self.diffs.poll_next_unpin(cx) {
            match diff {
                Ok(diff) => return Poll::Ready(Some(diff)),
                Err(()) => {
                    error!("couldn't fetch diff");
                }
            }
        }

        Poll::Pending
    }
}
