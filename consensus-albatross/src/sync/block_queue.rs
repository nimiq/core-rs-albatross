use std::{
    task::{Context, Poll},
    sync::Arc,
    collections::{BTreeMap, HashSet},
    pin::Pin,
};

use futures::stream::{BoxStream, Stream};
use pin_project::pin_project;

use nimiq_blockchain_albatross::{Blockchain, PushResult};
use nimiq_block_albatross::Block;
use nimiq_network_interface::network::Topic;
use nimiq_primitives::policy;
use nimiq_hash::Blake2bHash;

use super::request_component::RequestComponent;


#[derive(Clone, Debug, Default)]
pub struct BlockTopic;

impl Topic for BlockTopic {
    type Item = Block;

    fn topic(&self) -> String {
        "blocks".to_owned()
    }
}


pub type BlockStream = BoxStream<'static, Block>;


#[derive(Clone, Debug)]
pub struct BlockQueueConfig {
    /// Buffer size limit
    buffer_max: usize,

    /// How many blocks ahead we will buffer.
    window_max: u32,
}

impl Default for BlockQueueConfig {
    fn default() -> Self {
        Self {
            buffer_max: 4 * policy::BATCH_LENGTH as usize,
            window_max: 2 * policy::BATCH_LENGTH,
        }
    }
}


struct Inner {
    /// Configuration for the block queue
    config: BlockQueueConfig,

    /// Reference to the block chain
    blockchain: Arc<Blockchain>,

    /// Buffered blocks - `block_height -> [Block]`. There can be multiple blocks at a height if there are forks.
    ///
    /// # TODO
    ///
    ///  - The inner `Vec` should really be a `SmallVec<[Block; 1]>` or similar.
    ///
    buffer: BTreeMap<u32, Vec<Block>>,
}

impl Inner {
    fn on_block_announced<TReq: RequestComponent>(&mut self, block: Block, mut request_component: Pin<&mut TReq>) {
        let block_height = block.block_number();
        let head_height = self.blockchain.block_number();

        if block_height <= head_height {
            // Fork block
            self.push_block(block);
        }
        else if block_height == head_height + 1 {
            // New head block
            self.push_block(block);
            self.push_buffered();
        }
        else if block_height > head_height + self.config.window_max {
            log::warn!(
                "Discarding block #{} outside of buffer window (max {}).",
                 block_height,
                 head_height + self.config.window_max,
             );
        }
        else if self.buffer.len() >= self.config.buffer_max {
            log::warn!(
                "Discarding block #{}, buffer full (max {})",
                block_height,
                self.buffer.len(),
            )
        }
        else {
            let block_hash = block.hash();
            let head_hash = self.blockchain.head_hash();

            let prev_macro_block_height = policy::last_macro_block(head_height);

            // Put block inside buffer window
            self.insert_into_buffer(block);

            log::trace!("Requesting missing blocks: target_hash = {}, head_hash = {}, prev_macro_block_height = {}", block_hash, head_hash, prev_macro_block_height);
            log::trace!("n = {} = {} - {} + 1", head_height - prev_macro_block_height + 1, head_height, prev_macro_block_height);

            // get block locators
            let block_locators = self.blockchain
                .chain_store
                .get_blocks_backward(&head_hash, block_height - prev_macro_block_height + 1, false, None)
                .into_iter()
                .map(|block| block.hash())
                .collect::<Vec<Blake2bHash>>();

            log::trace!("block_locators = {:?}", block_locators);

            request_component.request_missing_blocks(block_hash, block_locators);
        }
    }

    fn on_missing_blocks_received(&mut self, blocks: Vec<Block>) {
        let mut it = blocks.into_iter();

        // Hashes of invalid blocks
        let mut invalid_blocks = HashSet::new();

        // Try to push blocks, until we encounter an inferior or invalid block
        while let Some(block) = it.next() {
            let block_hash = block.hash();

            log::trace!("Pushing block #{} from missing blocks response", block.block_number());

            match self.blockchain.push(block) {
                Ok(PushResult::Ignored) => {
                    log::warn!("Inferior chain - Aborting");
                    invalid_blocks.insert(block_hash);
                    break;
                },
                Err(e) => {
                    log::warn!("Failed to push block: {}", e);
                    invalid_blocks.insert(block_hash);
                    break;
                },
                Ok(result) => {
                    log::trace!("Block pushed: {:?}", result);
                },
            }
        }

        // If there are remaining blocks in the iterator, those are invalid.
        it.for_each(|block| { invalid_blocks.insert(block.hash()); });

        if !invalid_blocks.is_empty() {
            log::trace!("Removing any blocks that depend on: {:?}", invalid_blocks);

            // Iterate over all offsets, remove element if no blocks remain at that offset.
            self.buffer.drain_filter(|_block_number, blocks| {
                // Iterate over all blocks at an offset, remove block, if parent is invalid
                blocks.drain_filter(|block| {
                    if invalid_blocks.contains(block.parent_hash()) {
                        log::trace!("Removing block because parent is invalid: {}", block.hash());
                        invalid_blocks.insert(block.hash());
                        true
                    }
                    else {
                        false
                    }
                });
                blocks.is_empty()
            });
        }

        // We might be able to push buffered blocks now
        self.push_buffered();
    }

    fn push_block(&mut self, block: Block) {
        match self.blockchain.push(block) {
            Ok(result) => log::trace!("Block pushed: {:?}", result),
            Err(e) => log::warn!("Failed to push block: {}", e),
        }
    }

    fn push_buffered(&mut self) {
        loop {
            let head_height = self.blockchain.block_number();
            log::trace!("head_height = {}", head_height);

            // Check if queued block can be pushed to block chain
            if let Some(entry) = self.buffer.first_entry() {
                log::trace!("first entry: {:?}", entry);

                if *entry.key() > head_height + 1 {
                    break;
                }

                // Pop block from queue
                let (_, blocks) = entry.remove_entry();

                // If we get a Vec from the BTree, it must not be empty
                assert!(!blocks.is_empty());

                for block in blocks {
                    log::trace!(
                        "Pushing block #{} (currently at #{}, {} blocks left)",
                        block.block_number(),
                        head_height,
                        self.buffer.len(),
                    );
                    self.push_block(block);
                }
            }
            else {
                break;
            }
        }
    }

    fn insert_into_buffer(&mut self, block: Block) {
        self.buffer.entry(block.block_number())
            .or_default()
            .push(block)
    }
}


#[pin_project]
pub struct BlockQueue<TReq: RequestComponent> {
    /// The Peer Tracking and Request Component.
    #[pin]
    request_component: TReq,

    /// The blocks received via gossipsub.
    #[pin]
    block_stream: BlockStream,

    /// The inner state of the block queue.
    inner: Inner,
}

impl<TReq: RequestComponent> BlockQueue<TReq> {
    pub fn new(config: BlockQueueConfig, blockchain: Arc<Blockchain>, request_component: TReq, block_stream: BlockStream) -> Self {
        let buffer = BTreeMap::new();

        Self {
            request_component,
            block_stream,
            inner: Inner {
                config,
                blockchain,
                buffer,
            }
        }
    }

    /// Returns an iterator over the buffered blocks
    pub fn buffered_blocks(&self) -> impl Iterator<Item=(u32, &[Block])> {
        self.inner.buffer.iter().map(|(block_number, blocks)| (*block_number, blocks.as_ref()))
    }
}

impl<TReq: RequestComponent> Stream for BlockQueue<TReq> {
    type Item = ();

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let this = self.project();

        // Note: I think it doesn't matter what is done first

        // First, try to get as many blocks from the gossipsub stream as possible
        match this.block_stream.poll_next(cx) {
            Poll::Ready(Some(block)) => {
                this.inner.on_block_announced(block, this.request_component);
                return Poll::Ready(Some(()));
            },

            // If the block_stream is exhausted, we quit as well
            Poll::Ready(None) => return Poll::Ready(None),

            Poll::Pending => {},
        }

        // Then, read all the responses we got for our missing blocks requests
        match this.request_component.poll_next(cx) {
            Poll::Ready(Some(blocks)) => {
                this.inner.on_missing_blocks_received(blocks);
                return Poll::Ready(Some(()))
            },
            Poll::Ready(None) => panic!("The request_component stream is exhausted"),
            Poll::Pending => {},
        }

        Poll::Pending
    }
}
