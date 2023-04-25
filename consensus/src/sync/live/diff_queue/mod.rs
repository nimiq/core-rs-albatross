use std::{
    collections::HashSet,
    future::Future,
    ops,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    future::BoxFuture,
    stream::{BoxStream, FuturesOrdered, FuturesUnordered},
    Stream, StreamExt, TryStreamExt,
};
use parking_lot::RwLock;

use nimiq_block::Block;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::{AbstractBlockchain, BlockchainEvent};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{
    network::Network,
    request::{RequestCommon, RequestMarker},
};
use nimiq_primitives::{key_nibbles::KeyNibbles, trie::trie_diff::TrieDiff};
use nimiq_serde::{Deserialize, Serialize};

use super::block_queue::{BlockAndId, BlockQueue, QueuedBlock};

use self::diff_request_component::DiffRequestComponent;

pub mod diff_request_component;
pub mod live_sync;

/// The max number of partial trie diffs requests per peer.
pub const MAX_REQUEST_RESPONSE_PARTIAL_DIFFS: u32 = 100;

/// The request of a trie diff.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestPartialDiff {
    pub block_hash: Blake2bHash,
    pub range: ops::RangeTo<KeyNibbles>,
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
    F: FnMut(&Block) -> R,
    R: Future<Output = Result<TrieDiff, ()>>,
{
    async fn get_multiple_diffs<F, R>(
        blocks: &[&Block],
        mut get_diff: F,
    ) -> Result<Vec<(usize, TrieDiff)>, ()>
    where
        F: FnMut(&Block) -> R,
        R: Future<Output = Result<TrieDiff, ()>>,
    {
        // This is just a fancy way to collect all the diffs simultaneously.
        let diffs: FuturesUnordered<_> = blocks
            .iter()
            .enumerate()
            .map(|(i, &block)| {
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
        assert!(blocks.len() == diffs.len());
        Ok(diffs)
    }
    Ok(match block {
        QueuedBlock::Head((block, id)) => {
            let diff = get_diff(&block).await?;
            QueuedDiff::Head((block, id), diff)
        }
        QueuedBlock::Buffered(blocks) => {
            let diffs =
                get_multiple_diffs(&blocks.iter().map(|(b, _)| b).collect::<Vec<_>>(), get_diff)
                    .await?;
            assert!(blocks.len() == diffs.len());
            QueuedDiff::Buffered(
                blocks
                    .into_iter()
                    .zip(diffs.into_iter().map(|(_, d)| d))
                    .collect(),
            )
        }
        QueuedBlock::Missing(blocks) => {
            let diffs = get_multiple_diffs(&blocks.iter().collect::<Vec<_>>(), get_diff).await?;
            assert!(blocks.len() == diffs.len());
            QueuedDiff::Missing(
                blocks
                    .into_iter()
                    .zip(diffs.into_iter().map(|(_, d)| d))
                    .collect(),
            )
        }
        QueuedBlock::TooFarAhead(_, peer_id) => QueuedDiff::TooFarAhead(peer_id),
        QueuedBlock::TooFarBehind(_, peer_id) => QueuedDiff::TooFarBehind(peer_id),
    })
}

pub struct DiffQueue<N: Network> {
    /// Reference to the blockchain.
    blockchain: Arc<RwLock<Blockchain>>,

    /// The BlockQueue component.
    block_queue: BlockQueue<N>,

    /// The chunk request component.
    /// We use it to request chunks from up-to-date peers
    pub(crate) diff_request_component: DiffRequestComponent<N>,

    diffs: FuturesOrdered<BoxFuture<'static, Result<QueuedDiff<N>, ()>>>,

    blockchain_rx: BoxStream<'static, BlockchainEvent>,
}

impl<N: Network> DiffQueue<N> {
    pub fn with_block_queue(
        network: Arc<N>,
        blockchain: Arc<RwLock<Blockchain>>,
        block_queue: BlockQueue<N>,
    ) -> Self {
        let blockchain_rx = blockchain.read().notifier_as_stream();

        let diff_request_component = DiffRequestComponent::new(
            Arc::clone(&network),
            network.subscribe_events(),
            block_queue.request_component.peer_list(),
        );
        Self {
            blockchain,
            block_queue,
            diff_request_component,
            diffs: FuturesOrdered::new(),
            blockchain_rx,
        }
    }

    pub fn remove_invalid_blocks(&mut self, invalid_blocks: &mut HashSet<Blake2bHash>) {
        // We remove invalid blocks from the block queue.
        self.block_queue.remove_invalid_blocks(invalid_blocks);
    }

    pub fn on_block_processed(&mut self, hash: &Blake2bHash) {
        self.block_queue.on_block_processed(hash)
    }

    pub fn buffered_blocks_len(&self) -> usize {
        self.block_queue.buffered_blocks_len()
    }
}

impl<N: Network> Stream for DiffQueue<N> {
    type Item = QueuedDiff<N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // Poll the blockchain stream and set key start to complete if the accounts trie is
        // complete and we have passed a macro block.
        // Reset on a rebranch (since we potentially revert to an incomplete state).
        while let Poll::Ready(Some(event)) = self.blockchain_rx.poll_next_unpin(cx) {
            match event {
                BlockchainEvent::Finalized(_) | BlockchainEvent::EpochFinalized(_) => {
                    /*
                    let blockchain_state_complete =
                        self.blockchain.read().state.accounts.is_complete(None);
                    if !self.start_key.is_complete() && blockchain_state_complete {
                        // Mark state sync as complete after passing a macro block.
                        info!("Finished state sync, trie complete.");
                        self.start_key = ChunkRequestState::Complete;
                        self.buffer.clear();
                        self.buffer_size = 0;
                    } else if self.start_key.is_complete() && !blockchain_state_complete {
                        // Start state sync if the blockchain state was reinitialized after pushing a macro block.
                        info!("Trie incomplete, starting state sync.");
                        self.start_key = ChunkRequestState::Reset;
                    }
                    */
                }
                BlockchainEvent::Rebranched(_, _) => {
                    if !self.blockchain.read().state.accounts.is_complete(None) {
                        info!("Reset due to rebranch.");
                    }
                }
                _ => {}
            }
        }

        let mut block_queue_done = false;

        // 1. Receive blocks from BlockQueue.
        loop {
            match self.block_queue.poll_next_unpin(cx) {
                Poll::Ready(Some(block)) => {
                    let get_diff = self
                        .diff_request_component
                        .request_diff2(..KeyNibbles::ROOT);
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

        // 2. Check for blocks augmented with diffs.
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
