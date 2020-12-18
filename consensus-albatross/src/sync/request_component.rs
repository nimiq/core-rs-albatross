use crate::consensus_agent::ConsensusAgent;
use crate::sync::sync_queue::SyncQueue;
use block_albatross::Block;
use futures::stream::BoxStream;
use futures::task::{Context, Poll};
use futures::{FutureExt, Stream, StreamExt};
use hash::Blake2bHash;
use network_interface::peer::Peer;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug)]
/// Peer Tracking & Request Component
///
/// - Has sync queue
/// - Polls synced peers from history sync
/// - Puts peers to sync queue
/// - Removal happens automatically by the SyncQueue
///
/// Outside has a request blocks method, which doesn’t return the blocks.
/// The blocks instead are returned by polling the component.
pub struct BlockRequestComponent<TPeer: Peer> {
    sync_queue: SyncQueue<TPeer, (Blake2bHash, Vec<Blake2bHash>), Vec<Block>>, // requesting missing blocks from peers
    sync_method: BoxStream<'static, Arc<ConsensusAgent<TPeer>>>,
}

impl<TPeer: Peer> BlockRequestComponent<TPeer> {
    const NUM_PENDING_BLOCKS: usize = 5;

    pub fn new(sync_method: BoxStream<'static, Arc<ConsensusAgent<TPeer>>>) -> Self {
        Self {
            sync_method,
            sync_queue: SyncQueue::new(
                vec![],
                vec![],
                Self::NUM_PENDING_BLOCKS,
                |(target_block_hash, locators), peer| {
                    async move {
                        peer.request_missing_blocks(target_block_hash, locators)
                            .await
                            .ok()
                    }
                    .boxed()
                },
            ),
        }
    }

    fn request_missing_blocks(
        &mut self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    ) {
        self.sync_queue.add_ids(vec![(target_block_hash, locators)]);
    }
}

impl<TPeer: Peer> Stream for BlockRequestComponent<TPeer> {
    type Item = Vec<Block>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        // 1. Poll self.sync_method and add new peers to self.sync_queue.
        while let Poll::Ready(Some(result)) = self.sync_method.poll_next_unpin(cx) {
            self.sync_queue.add_peer(Arc::downgrade(&result));
        }

        // 2. Poll self.sync_queue, return results.
        while let Poll::Ready(Some(result)) = self.sync_queue.poll_next_unpin(cx) {
            match result {
                Ok(blocks) => return Poll::Ready(Some(blocks)),
                Err((target_hash, _)) => {
                    debug!(
                        "Failed to retrieve missing blocks for target hash {}",
                        target_hash
                    );
                    // TODO: Do we need to do anything else?
                }
            }
        }

        Poll::Pending
    }
}
