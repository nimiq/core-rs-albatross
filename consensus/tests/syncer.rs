use std::collections::HashSet;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{poll, Stream, StreamExt};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::messages::{RequestMissingBlocks, ResponseBlocks};
use nimiq_consensus::sync::live::block_queue::block_request_component::BlockRequestComponent;
use nimiq_consensus::sync::live::queue::QueueConfig;
use nimiq_network_interface::request::RequestCommon;
use parking_lot::{Mutex, RwLock};
use pin_project::pin_project;
use rand::Rng;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use nimiq_block::Block;
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, Direction};
use nimiq_consensus::sync::live::block_queue::BlockQueue;
use nimiq_consensus::sync::live::BlockLiveSync;
use nimiq_consensus::sync::syncer::{LiveSync, MacroSync, MacroSyncReturn, Syncer};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network;
use nimiq_network_mock::{MockHub, MockId, MockPeerId};
use nimiq_primitives::networks::NetworkId;
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{
        next_micro_block, produce_macro_blocks, push_micro_block, signing_key, voting_key,
    },
    mock_node::MockNode,
    node::TESTING_BLS_CACHE_MAX_CAPACITY,
};
use nimiq_utils::time::OffsetTime;

#[pin_project]
#[derive(Debug)]
pub struct MockRequestComponent {
    pub peers: HashSet<MockPeerId>,
    pub tx: mpsc::UnboundedSender<(Blake2bHash, Vec<Blake2bHash>)>,
    #[pin]
    pub rx: mpsc::UnboundedReceiver<Vec<Block>>,
}

impl MockRequestComponent {
    pub fn new_mutex() -> (
        Self,
        mpsc::UnboundedReceiver<(Blake2bHash, Vec<Blake2bHash>)>,
        mpsc::UnboundedSender<Vec<Block>>,
    ) {
        let (tx1, rx1) = mpsc::unbounded_channel();
        let (tx2, rx2) = mpsc::unbounded_channel();

        (
            Self {
                peers: Default::default(),
                tx: tx1,
                rx: rx2,
            },
            rx1,
            tx2,
        )
    }

    pub fn new() -> Self {
        MockRequestComponent::new_mutex().0
    }
}

#[derive(Default)]
struct MockHistorySyncStream {
    pub peers: Arc<RwLock<Vec<MockPeerId>>>,
}

impl MockHistorySyncStream {
    pub fn new() -> MockHistorySyncStream {
        Default::default()
    }
}

impl Stream for MockHistorySyncStream {
    type Item = MacroSyncReturn<MockPeerId>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

impl MacroSync<MockPeerId> for MockHistorySyncStream {
    fn add_peer(&self, peer_id: MockPeerId) {
        self.peers.write().push(peer_id);
    }
}

fn blockchain() -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
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

fn bls_cache() -> Arc<Mutex<PublicKeyCache>> {
    Arc::new(Mutex::new(PublicKeyCache::new(
        TESTING_BLS_CACHE_MAX_CAPACITY,
    )))
}

#[test(tokio::test)]
async fn send_single_micro_block_to_block_queue() {
    let blockchain2 = blockchain();
    let blockchain_proxy = BlockchainProxy::from(&blockchain2);

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let request_component =
        BlockRequestComponent::new(network.subscribe_events(), Arc::clone(&network), true);
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy.clone(),
        Arc::clone(&network),
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy.clone(),
        Arc::clone(&network),
        block_queue,
        bls_cache(),
    );
    let mut syncer = Syncer::new(live_sync, MockHistorySyncStream::new());

    let mock_node =
        MockNode::with_network_and_blockchain(Arc::new(hub.new_network()), blockchain());
    network.dial_mock(&mock_node.network);
    syncer
        .live_sync
        .add_peer(mock_node.network.get_local_peer_id());

    // push one micro block to the queue
    let producer = BlockProducer::new(signing_key(), voting_key());
    let block = next_micro_block(&producer, &blockchain2);

    let mock_id = MockId::new(mock_node.network.get_local_peer_id());
    block_tx.send((block, mock_id)).await.unwrap();

    assert_eq!(blockchain2.read().block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    syncer.next().await;

    // The produced block is without gap and should go right into the blockchain
    assert_eq!(blockchain2.read().block_number(), 1);
    assert!(syncer.live_sync.queue().buffered_blocks().next().is_none());
}

#[test(tokio::test)]
async fn send_two_micro_blocks_out_of_order() {
    let blockchain1 = blockchain();
    let blockchain_proxy_1 = BlockchainProxy::from(&blockchain1);
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let request_component =
        BlockRequestComponent::new(network.subscribe_events(), Arc::clone(&network), true);

    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        bls_cache(),
    );

    let mut syncer = Syncer::new(live_sync, MockHistorySyncStream::new());

    let mut mock_node =
        MockNode::with_network_and_blockchain(Arc::new(hub.new_network()), blockchain());
    network.dial_mock(&mock_node.network);
    syncer
        .live_sync
        .add_peer(mock_node.network.get_local_peer_id());

    let producer = BlockProducer::new(signing_key(), voting_key());
    let block1 = push_micro_block(&producer, &blockchain2);
    let block2 = next_micro_block(&producer, &blockchain2);

    let mock_id = MockId::new(mock_node.network.get_local_peer_id());

    // send block2 first
    block_tx
        .send((block2.clone(), mock_id.clone()))
        .await
        .unwrap();

    assert_eq!(blockchain1.read().block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    let _ = poll!(syncer.next());

    // this block should be buffered now
    assert_eq!(blockchain1.read().block_number(), 0);
    let blocks = syncer
        .live_sync
        .queue()
        .buffered_blocks()
        .collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, 2);
    assert_eq!(blocks[0], &block2);

    // Also we should've received a request to fill this gap
    let req = mock_node.next().await.unwrap();
    assert_eq!(req, RequestMissingBlocks::TYPE_ID);

    // now send block1 to fill the gap
    block_tx.send((block1.clone(), mock_id)).await.unwrap();

    // run the block_queue until is has produced two events.
    syncer.next().await;
    syncer.next().await;

    // now both blocks should've been pushed to the blockchain
    assert_eq!(blockchain1.read().block_number(), 2);
    assert!(syncer.live_sync.queue().buffered_blocks().next().is_none());
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
    let blockchain_proxy_1 = BlockchainProxy::from(&blockchain1);
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let request_component =
        BlockRequestComponent::new(network.subscribe_events(), Arc::clone(&network), true);
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        bls_cache(),
    );
    let mut syncer = Syncer::new(live_sync, MockHistorySyncStream::new());

    let mock_node =
        MockNode::with_network_and_blockchain(Arc::new(hub.new_network()), blockchain());
    network.dial_mock(&mock_node.network);
    syncer
        .live_sync
        .add_peer(mock_node.network.get_local_peer_id());

    let mut rng = rand::thread_rng();
    let mut ordered_blocks = Vec::new();

    let mock_id = MockId::new(mock_node.network.get_local_peer_id());
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
        let _ = poll!(syncer.next());
    }

    // All blocks should be buffered
    assert_eq!(blockchain1.read().block_number(), 0);

    // Obtain the buffered blocks
    assert_eq!(
        syncer.live_sync.queue().buffered_blocks().count() as u64,
        n_blocks - 1
    );

    // now send block1 to fill the gap
    block_tx.send((blocks[0].clone(), mock_id)).await.unwrap();

    for _ in 0..n_blocks {
        syncer.next().await;
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
    assert!(syncer.live_sync.queue().buffered_blocks().next().is_none());
}

#[test(tokio::test)]
async fn send_invalid_block() {
    let blockchain1 = blockchain();
    let blockchain_proxy_1 = BlockchainProxy::from(&blockchain1);
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let request_component =
        BlockRequestComponent::new(network.subscribe_events(), Arc::clone(&network), true);
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        bls_cache(),
    );
    let mut syncer = Syncer::new(live_sync, MockHistorySyncStream::new());

    let mut mock_node =
        MockNode::with_network_and_blockchain(Arc::new(hub.new_network()), blockchain());
    network.dial_mock(&mock_node.network);
    syncer
        .live_sync
        .add_peer(mock_node.network.get_local_peer_id());

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
    let _ = poll!(syncer.next());

    // this block should be buffered now
    assert_eq!(blockchain1.read().block_number(), 0);
    let blocks = syncer
        .live_sync
        .queue()
        .buffered_blocks()
        .collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, 2);
    assert_eq!(blocks[0], &block2);

    let req = mock_node.next().await.unwrap();
    assert_eq!(req, RequestMissingBlocks::TYPE_ID);

    // now send block1 to fill the gap
    block_tx.send((block1.clone(), mock_id)).await.unwrap();

    // run the block_queue until is has produced two events.
    // The second block will be rejected due to an Invalid Successor event
    syncer.next().await;
    syncer.next().await;

    // Only Block 1 should be pushed to the blockchain
    assert_eq!(blockchain1.read().block_number(), 1);
    assert!(syncer.live_sync.queue().buffered_blocks().next().is_none());
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
    let blockchain_proxy_1 = BlockchainProxy::from(&blockchain1);

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let request_component =
        BlockRequestComponent::new(network.subscribe_events(), Arc::clone(&network), true);
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        bls_cache(),
    );
    let mut syncer = Syncer::new(live_sync, MockHistorySyncStream::new());

    let mut mock_node =
        MockNode::with_network_and_blockchain(Arc::new(hub.new_network()), blockchain());
    network.dial_mock(&mock_node.network);
    syncer
        .live_sync
        .add_peer(mock_node.network.get_local_peer_id());

    let producer = BlockProducer::new(signing_key(), voting_key());
    let block1 = push_micro_block(&producer, &mock_node.blockchain);
    let block2 = next_micro_block(&producer, &mock_node.blockchain);

    let mock_id = MockId::new(mock_node.network.get_local_peer_id());

    // send block2 first
    block_tx.send((block2.clone(), mock_id)).await.unwrap();

    assert_eq!(blockchain1.read().block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    let _ = poll!(syncer.next());

    // this block should be buffered now
    assert_eq!(blockchain1.read().block_number(), 0);
    let blocks = syncer
        .live_sync
        .queue()
        .buffered_blocks()
        .collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, 2);
    assert_eq!(blocks[0], &block2);

    // Also we should've received a request to fill this gap
    // Instead of gossiping the block, we'll answer the missing blocks request
    let req = mock_node.next().await.unwrap();
    assert_eq!(req, RequestMissingBlocks::TYPE_ID);

    // run the block_queue until is has produced two events.
    syncer.next().await;
    syncer.next().await;

    // now both blocks should've been pushed to the blockchain
    assert_eq!(blockchain1.read().block_number(), 2);
    assert!(syncer.live_sync.queue().buffered_blocks().next().is_none());
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
    let blockchain_proxy_1 = BlockchainProxy::from(&blockchain1);

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let request_component =
        BlockRequestComponent::new(network.subscribe_events(), Arc::clone(&network), true);
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        bls_cache(),
    );
    let mut syncer = Syncer::new(live_sync, MockHistorySyncStream::new());

    let mut mock_node =
        MockNode::with_network_and_blockchain(Arc::new(hub.new_network()), blockchain());
    network.dial_mock(&mock_node.network);
    syncer
        .live_sync
        .add_peer(mock_node.network.get_local_peer_id());

    let producer = BlockProducer::new(signing_key(), voting_key());
    produce_macro_blocks(&producer, &mock_node.blockchain, 1);
    let block1 = push_micro_block(&producer, &mock_node.blockchain);
    let block2 = push_micro_block(&producer, &mock_node.blockchain);

    let mock_id = MockId::new(mock_node.network.get_local_peer_id());

    // send block2 first
    block_tx.send((block2.clone(), mock_id)).await.unwrap();

    assert_eq!(blockchain1.read().block_number(), 0);

    // run the block_queue one iteration, i.e. until it processed one block
    let _ = poll!(syncer.next());

    // this block should be buffered now
    assert_eq!(blockchain1.read().block_number(), 0);
    let blocks = syncer
        .live_sync
        .queue()
        .buffered_blocks()
        .collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, block2.block_number());
    assert_eq!(blocks[0], &block2);

    // Also we should've received a request to fill the first gap.
    // Instead of gossiping the block, we'll answer the missing blocks request
    mock_node.set_missing_block_handler(Some(|req, blockchain| {
        assert_eq!(&req.target_hash, blockchain.read().head().parent_hash());

        let blocks = blockchain
            .read()
            .get_blocks(
                &blockchain.read().macro_head().header.parent_hash,
                2,
                true,
                Direction::Forward,
            )
            .unwrap();

        ResponseBlocks {
            blocks: Some(blocks),
        }
    }));
    let req = mock_node.next().await.unwrap();
    assert_eq!(req, RequestMissingBlocks::TYPE_ID);

    let macro_head = Block::Macro(mock_node.blockchain.read().macro_head());

    // Run the block_queue one iteration, i.e. until it processed one block
    let _ = poll!(syncer.next());

    // The blocks from the first missing blocks request should be buffered now
    assert_eq!(blockchain1.read().block_number(), 0);
    let blocks = syncer
        .live_sync
        .queue()
        .buffered_blocks()
        .collect::<Vec<_>>();
    assert_eq!(blocks.len(), 3);
    let (block_number, blocks) = blocks.get(0).unwrap();
    assert_eq!(*block_number, macro_head.block_number());
    assert_eq!(blocks[0], &macro_head);

    // Also we should've received a request to fill the second gap.
    mock_node.set_missing_block_handler(None);
    let req = mock_node.next().await.unwrap();
    assert_eq!(req, RequestMissingBlocks::TYPE_ID);

    // Run the block_queue until is has produced four events:
    //   - ReceivedMissingBlocks (1-31)
    //   - AcceptedBufferedBlock (32)
    //   - AcceptedBufferedBlock (33)
    //   - AcceptedBufferedBlock (34)
    syncer.next().await;
    syncer.next().await;
    syncer.next().await;
    syncer.next().await;

    // Now all blocks should've been pushed to the blockchain.
    assert_eq!(blockchain1.read().block_number(), block2.block_number());
    assert!(syncer.live_sync.queue().buffered_blocks().next().is_none());
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
    let blockchain_proxy_1 = BlockchainProxy::from(&blockchain1);
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let request_component =
        BlockRequestComponent::new(network.subscribe_events(), Arc::clone(&network), true);
    let history_sync = MockHistorySyncStream::new();
    let history_sync_peers = history_sync.peers.clone();
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig {
            buffer_max: 10,
            window_ahead_max: 10,
            tolerate_past_max: 100,
            include_micro_bodies: true,
        },
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        bls_cache(),
    );
    let mut syncer = Syncer::new(live_sync, history_sync);

    let mock_node =
        MockNode::with_network_and_blockchain(Arc::new(hub.new_network()), blockchain());
    network.dial_mock(&mock_node.network);
    syncer
        .live_sync
        .add_peer(mock_node.network.get_local_peer_id());
    let mock_id = MockId::new(mock_node.network.get_local_peer_id());

    let producer = BlockProducer::new(signing_key(), voting_key());
    for _ in 1..11 {
        push_micro_block(&producer, &blockchain2);
    }

    let block = next_micro_block(&producer, &blockchain2);
    block_tx.send((block, mock_id)).await.unwrap();

    // run the block_queue one iteration, i.e. until it processed one block
    let _ = poll!(syncer.next());

    assert_eq!(history_sync_peers.read().len(), 1);
}
