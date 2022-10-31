use std::{
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{task::noop_waker_ref, Stream, StreamExt};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use parking_lot::{Mutex, RwLock};
use pin_project::pin_project;
use rand::Rng;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use nimiq_block::Block;
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain, Direction};
use nimiq_consensus::sync::block_queue::{BlockQueue, BlockQueueConfig};
use nimiq_consensus::sync::request_component::{RequestComponent, RequestComponentEvent};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;
use nimiq_network_mock::{MockHub, MockId, MockNetwork};
use nimiq_primitives::networks::NetworkId;
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{
        next_micro_block, produce_macro_blocks, push_micro_block, signing_key, voting_key,
    },
    node::TESTING_BLS_CACHE_MAX_CAPACITY,
};
use nimiq_utils::time::OffsetTime;

#[pin_project]
#[derive(Debug)]
pub struct MockRequestComponent<P> {
    pub peer_put_into_sync: bool,
    pub tx: mpsc::UnboundedSender<(Blake2bHash, Vec<Blake2bHash>)>,
    #[pin]
    pub rx: mpsc::UnboundedReceiver<Vec<Block>>,
    peer_type: PhantomData<P>,
}

impl<P> MockRequestComponent<P> {
    pub fn new() -> (
        Self,
        mpsc::UnboundedReceiver<(Blake2bHash, Vec<Blake2bHash>)>,
        mpsc::UnboundedSender<Vec<Block>>,
    ) {
        let (tx1, rx1) = mpsc::unbounded_channel();
        let (tx2, rx2) = mpsc::unbounded_channel();

        (
            Self {
                peer_put_into_sync: false,
                tx: tx1,
                rx: rx2,
                peer_type: PhantomData,
            },
            rx1,
            tx2,
        )
    }
}

impl<N: Network> RequestComponent<N> for MockRequestComponent<N> {
    fn request_missing_blocks(
        &mut self,
        target_block_hash: Blake2bHash,
        locators: Vec<Blake2bHash>,
    ) {
        if self.tx.send((target_block_hash, locators)).is_err() {
            log::error!(
                error = "receiver hung up",
                "error requesting missing blocks",
            );
        }
    }

    fn put_peer_into_sync_mode(&mut self, _peer: N::PeerId) {
        self.peer_put_into_sync = true;
    }

    fn num_peers(&self) -> usize {
        1
    }

    fn peers(&self) -> Vec<N::PeerId> {
        unimplemented!()
    }
}

impl<N> Default for MockRequestComponent<N> {
    fn default() -> Self {
        Self::new().0
    }
}

impl<N: Network> Stream for MockRequestComponent<N> {
    type Item = RequestComponentEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match self.project().rx.poll_recv(cx) {
            Poll::Ready(Some(blocks)) => {
                Poll::Ready(Some(RequestComponentEvent::ReceivedBlocks(blocks)))
            }
            _ => Poll::Pending,
        }
    }
}

fn blockchain() -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    Arc::new(RwLock::new(
        Blockchain::new(env, NetworkId::UnitAlbatross, time).unwrap(),
    ))
}

#[test(tokio::test)]
async fn send_single_micro_block_to_block_queue() {
    let blockchain = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let request_component = MockRequestComponent::<MockNetwork>::default();
    let (block_tx, block_rx) = mpsc::channel(32);

    let mut block_queue = BlockQueue::with_block_stream(
        Default::default(),
        BlockchainProxy::from(&blockchain),
        Arc::clone(&network),
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
    );

    // push one micro block to the queue
    let producer = BlockProducer::new(signing_key(), voting_key());
    let block = next_micro_block(&producer, &blockchain);

    let mock_id = MockId::new(hub.new_address().into());
    block_tx.send((block, mock_id)).await.unwrap();

    assert_eq!(blockchain.read().block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    block_queue.next().await;

    // The produced block is without gap and should go right into the blockchain
    assert_eq!(blockchain.read().block_number(), 1);
    assert!(block_queue.buffered_blocks().next().is_none());
}

#[test(tokio::test)]
async fn send_two_micro_blocks_out_of_order() {
    let blockchain1 = blockchain();
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let (request_component, mut missing_blocks_request_rx, _) =
        MockRequestComponent::<MockNetwork>::new();
    let (block_tx, block_rx) = mpsc::channel(32);

    let mut block_queue = BlockQueue::with_block_stream(
        Default::default(),
        BlockchainProxy::from(&blockchain1),
        network,
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
    );

    let producer = BlockProducer::new(signing_key(), voting_key());
    let block1 = push_micro_block(&producer, &blockchain2);
    let block2 = next_micro_block(&producer, &blockchain2);

    let mock_id = MockId::new(hub.new_address().into());

    // send block2 first
    block_tx
        .send((block2.clone(), mock_id.clone()))
        .await
        .unwrap();

    assert_eq!(blockchain1.read().block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    let _ = block_queue.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()));

    // this block should be buffered now
    assert_eq!(blockchain1.read().block_number(), 0);
    let blocks = block_queue.buffered_blocks().collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, 2);
    assert_eq!(blocks[0], &block2);

    // Also we should've received a request to fill this gap
    let (target_block_hash, _) = missing_blocks_request_rx.recv().await.unwrap();
    assert_eq!(&target_block_hash, block2.parent_hash());

    // now send block1 to fill the gap
    block_tx.send((block1.clone(), mock_id)).await.unwrap();

    // run the block_queue until is has produced two events.
    block_queue.next().await;
    block_queue.next().await;

    // now both blocks should've been pushed to the blockchain
    assert_eq!(blockchain1.read().block_number(), 2);
    assert!(block_queue.buffered_blocks().next().is_none());
    assert_eq!(
        blockchain1.read().get_block_at(1, true, None).unwrap(),
        block1
    );
    assert_eq!(
        blockchain1.read().get_block_at(2, true, None).unwrap(),
        block2
    );
}

#[test(tokio::test)]
async fn send_micro_blocks_out_of_order() {
    let blockchain1 = blockchain();
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let request_component = MockRequestComponent::<MockNetwork>::default();
    let (block_tx, block_rx) = mpsc::channel(32);

    let mut block_queue = BlockQueue::with_block_stream(
        Default::default(),
        BlockchainProxy::from(&blockchain1),
        network,
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
    );

    let mut rng = rand::thread_rng();
    let mut ordered_blocks = Vec::new();

    let mock_id = MockId::new(hub.new_address().into());
    let producer = BlockProducer::new(signing_key(), voting_key());
    let n_blocks = rng.gen_range(2..15);

    for _ in 0..n_blocks {
        let block = push_micro_block(&producer, &blockchain2);
        ordered_blocks.push(block);
    }

    let mut blocks = ordered_blocks.clone();

    while blocks.len() > 1 {
        let index = rng.gen_range(1..blocks.len());

        block_tx
            .send((blocks.remove(index).clone(), mock_id.clone()))
            .await
            .unwrap();

        // run the block_queue one iteration, i.e. until it processed one block
        let _ = block_queue.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()));
    }

    // All blocks should be buffered
    assert_eq!(blockchain1.read().block_number(), 0);

    // Obtain the buffered blocks
    assert_eq!(block_queue.buffered_blocks().count() as u64, n_blocks - 1);

    // now send block1 to fill the gap
    block_tx.send((blocks[0].clone(), mock_id)).await.unwrap();

    for _ in 0..n_blocks {
        block_queue.next().await;
    }

    // Verify all blocks except the genesis
    for i in 1..=n_blocks {
        assert_eq!(
            blockchain1
                .read()
                .get_block_at(i as u32, true, None)
                .unwrap(),
            ordered_blocks[(i - 1) as usize]
        );
    }

    // No blocks buffered
    assert!(block_queue.buffered_blocks().next().is_none());
}

#[test(tokio::test)]
async fn send_invalid_block() {
    let blockchain1 = blockchain();
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let (request_component, mut missing_blocks_request_rx, _) =
        MockRequestComponent::<MockNetwork>::new();
    let (block_tx, block_rx) = mpsc::channel(32);

    let mut block_queue = BlockQueue::with_block_stream(
        Default::default(),
        BlockchainProxy::from(&blockchain1),
        network,
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
    );

    let producer = BlockProducer::new(signing_key(), voting_key());
    let block1 = push_micro_block(&producer, &blockchain2);

    // Block2's timestamp is less than Block1's timestamp, so Block 2 will be rejected by the blockchain
    let block2 = {
        let mut block = next_micro_block(&producer, &blockchain2).unwrap_micro();
        block.header.timestamp = block1.timestamp() - 5;
        Block::Micro(block)
    };

    let mock_id = MockId::new(hub.new_address().into());

    // send block2 first
    block_tx
        .send((block2.clone(), mock_id.clone()))
        .await
        .unwrap();

    assert_eq!(blockchain1.read().block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    let _ = block_queue.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()));

    // this block should be buffered now
    assert_eq!(blockchain1.read().block_number(), 0);
    let blocks = block_queue.buffered_blocks().collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, 2);
    assert_eq!(blocks[0], &block2);

    let (target_block_hash, _locators) = missing_blocks_request_rx.recv().await.unwrap();
    assert_eq!(&target_block_hash, block2.parent_hash());

    // now send block1 to fill the gap
    block_tx.send((block1.clone(), mock_id)).await.unwrap();

    // run the block_queue until is has produced two events.
    // The second block will be rejected due to an Invalid Sucessor event
    block_queue.next().await;
    block_queue.next().await;

    // Only Block 1 should be pushed to the blockchain
    assert_eq!(blockchain1.read().block_number(), 1);
    assert!(block_queue.buffered_blocks().next().is_none());
    assert_eq!(
        blockchain1.read().get_block_at(1, true, None).unwrap(),
        block1
    );
    assert_ne!(
        blockchain1.read().get_block_at(1, true, None).unwrap(),
        block2
    );
}

#[test(tokio::test)]
async fn send_block_with_gap_and_respond_to_missing_request() {
    let blockchain1 = blockchain();
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network_with_address(1));
    let (request_component, mut missing_block_request_rx, missing_blocks_request_tx) =
        MockRequestComponent::<MockNetwork>::new();
    let (block_tx, block_rx) = mpsc::channel(32);

    let mut block_queue = BlockQueue::with_block_stream(
        Default::default(),
        BlockchainProxy::from(&blockchain1),
        network,
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
    );

    let producer = BlockProducer::new(signing_key(), voting_key());
    let block1 = push_micro_block(&producer, &blockchain2);
    let block2 = next_micro_block(&producer, &blockchain2);

    let mock_id = MockId::new(hub.new_address().into());

    // send block2 first
    block_tx.send((block2.clone(), mock_id)).await.unwrap();

    assert_eq!(blockchain1.read().block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    let _ = block_queue.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()));

    // this block should be buffered now
    assert_eq!(blockchain1.read().block_number(), 0);
    let blocks = block_queue.buffered_blocks().collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, 2);
    assert_eq!(blocks[0], &block2);

    // Also we should've received a request to fill this gap
    // TODO: Check block locators
    let (target_block_hash, _) = missing_block_request_rx.recv().await.unwrap();
    assert_eq!(&target_block_hash, block2.parent_hash());

    // Instead of gossiping the block, we'll answer the missing blocks request
    missing_blocks_request_tx
        .send(vec![block1.clone()])
        .unwrap();

    // run the block_queue until is has produced two events.
    block_queue.next().await;
    block_queue.next().await;

    // now both blocks should've been pushed to the blockchain
    assert_eq!(blockchain1.read().block_number(), 2);
    assert!(block_queue.buffered_blocks().next().is_none());
    assert_eq!(
        blockchain1.read().get_block_at(1, true, None).unwrap(),
        block1
    );
    assert_eq!(
        blockchain1.read().get_block_at(2, true, None).unwrap(),
        block2
    );
}

#[test(tokio::test)]
async fn request_missing_blocks_across_macro_block() {
    let blockchain1 = blockchain();
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network_with_address(1));
    let (request_component, mut missing_block_request_rx, missing_blocks_request_tx) =
        MockRequestComponent::<MockNetwork>::new();
    let (block_tx, block_rx) = mpsc::channel(32);

    let mut block_queue = BlockQueue::with_block_stream(
        Default::default(),
        BlockchainProxy::from(&blockchain1),
        network,
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
    );

    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(&producer, &blockchain2, 1);
    let block1 = push_micro_block(&producer, &blockchain2);
    let block2 = next_micro_block(&producer, &blockchain2);

    let mock_id = MockId::new(hub.new_address().into());

    // send block2 first
    block_tx.send((block2.clone(), mock_id)).await.unwrap();

    assert_eq!(blockchain1.read().block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    let _ = block_queue.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()));

    // this block should be buffered now
    assert_eq!(blockchain1.read().block_number(), 0);
    let blocks = block_queue.buffered_blocks().collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, block2.block_number());
    assert_eq!(blocks[0], &block2);

    // Also we should've received a request to fill the first gap.
    // TODO: Check block locators
    let (target_block_hash, _) = missing_block_request_rx.recv().await.unwrap();
    assert_eq!(&target_block_hash, block2.parent_hash());

    // Instead of gossiping the block, we'll answer the missing blocks request
    let macro_head = Block::Macro(blockchain2.read().macro_head());
    missing_blocks_request_tx
        .send(vec![macro_head.clone(), block1.clone()])
        .unwrap();

    // Run the block_queue one iteration, i.e. until it processed one block
    let _ = block_queue.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()));

    // The blocks from the first missing blocks request should be buffered now
    assert_eq!(blockchain1.read().block_number(), 0);
    let blocks = block_queue.buffered_blocks().collect::<Vec<_>>();
    assert_eq!(blocks.len(), 3);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, macro_head.block_number());
    assert_eq!(blocks[0], &macro_head);

    // Also we should've received a request to fill the second gap.
    // TODO: Check block locators
    let (target_block_hash, _) = missing_block_request_rx.recv().await.unwrap();
    assert_eq!(&target_block_hash, macro_head.parent_hash());

    // Respond to second missing blocks request.
    let target_block = blockchain2
        .read()
        .get_block(&target_block_hash, true, None)
        .unwrap();
    let mut blocks = blockchain2.read().get_blocks(
        &target_block_hash,
        macro_head.block_number() - 2,
        true,
        Direction::Backward,
    );
    blocks.reverse();
    blocks.push(target_block);
    missing_blocks_request_tx.send(blocks).unwrap();

    // Run the block_queue until is has produced four events:
    //   - ReceivedMissingBlocks (1-31)
    //   - AcceptedBufferedBlock (32)
    //   - AcceptedBufferedBlock (33)
    //   - AcceptedBufferedBlock (34)
    block_queue.next().await;
    block_queue.next().await;
    block_queue.next().await;
    block_queue.next().await;

    // Now all blocks should've been pushed to the blockchain.
    assert_eq!(blockchain1.read().block_number(), block2.block_number());
    assert!(block_queue.buffered_blocks().next().is_none());
    assert_eq!(
        blockchain1
            .read()
            .get_block_at(block1.block_number(), true, None)
            .unwrap(),
        block1
    );
    assert_eq!(
        blockchain1
            .read()
            .get_block_at(block2.block_number(), true, None)
            .unwrap(),
        block2
    );
}

#[test(tokio::test)]
async fn put_peer_back_into_sync_mode() {
    let blockchain1 = blockchain();
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network_with_address(1));
    let request_component = MockRequestComponent::<MockNetwork>::default();
    let (block_tx, block_rx) = mpsc::channel(32);

    let peer_addr = hub.new_address().into();
    let mock_id = MockId::new(peer_addr);
    network.dial_peer(peer_addr).await.unwrap();

    let mut block_queue = BlockQueue::with_block_stream(
        BlockQueueConfig {
            buffer_max: 10,
            window_max: 10,
        },
        BlockchainProxy::from(&blockchain1),
        network,
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
    );

    let producer = BlockProducer::new(signing_key(), voting_key());
    for _ in 1..11 {
        push_micro_block(&producer, &blockchain2);
    }

    let block = next_micro_block(&producer, &blockchain2);
    block_tx.send((block, mock_id)).await.unwrap();

    // run the block_queue one iteration, i.e. until it processed one block
    let _ = block_queue.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()));

    assert!(block_queue.request_component().peer_put_into_sync);
}
