use std::{
    collections::HashSet,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{poll, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, Direction};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::{
    messages::{RequestMissingBlocks, ResponseBlocks},
    sync::{
        live::{block_queue::BlockQueue, queue::QueueConfig, BlockLiveSync},
        sync_interface::{LiveSync, LiveSyncPushEvent, MacroSync, MacroSyncReturn},
        syncer::Syncer,
    },
    BlsCache,
};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{network::Network, request::RequestCommon};
use nimiq_network_mock::{MockHub, MockId, MockNetwork, MockPeerId};
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{
        next_micro_block, produce_macro_blocks, push_micro_block, signing_key, voting_key,
    },
    mock_node::MockNode,
    test_rng,
};
use nimiq_utils::time::OffsetTime;
use parking_lot::{Mutex, RwLock};
use rand::RngExt;
use tokio::{
    sync::mpsc::{self, error::TrySendError},
    task::yield_now,
};
use tokio_stream::wrappers::ReceiverStream;

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
    const MAX_REQUEST_EPOCHS: u16 = 3;

    fn add_peer(&mut self, peer_id: MockPeerId) {
        self.peers.write().push(peer_id);
    }

    fn drain_peers(&mut self) -> Vec<MockPeerId> {
        unimplemented!()
    }

    fn fallback(&mut self, _peers: Vec<MockPeerId>) {
        unimplemented!()
    }
}

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

/// Announcements fed before giving up. Well above any sensible channel capacity.
const MAX_ANNOUNCED_BLOCKS: usize = 4096;

/// Scheduler rounds the proxy's background task gets to drain before we call it backpressure.
const DRAIN_ROUNDS: usize = 16;

/// Announcements used for the delivery test. Must exceed the proxy's channel capacity.
const NUM_DELIVERED_BLOCKS: usize = 512;

/// Builds a distinct announcement by rewriting the header's extra data. Announcements are admitted
/// on `verify_header` alone, which checks no signature for micro blocks. The cached hash has to be
/// cleared, otherwise the queue deduplicates the variants.
fn forge_variant(template: &Block, nonce: u32) -> Block {
    let mut block = template.clone();
    match &mut block {
        Block::Micro(micro) => {
            micro.header.extra_data = nonce.to_be_bytes().to_vec();
            micro.header.cached_hash = None;
        }
        Block::Macro(_) => unreachable!("template is a micro block"),
    }
    block
}

/// Sets up a syncer taking announcements from `block_rx`, plus the peer that announces them. The
/// block queue only looks at announcements while it has at least one peer.
fn setup_live_sync(
    hub: &mut MockHub,
    chain: &Arc<RwLock<Blockchain>>,
    block_rx: mpsc::Receiver<(Block, MockId<MockPeerId>)>,
) -> (
    Syncer<MockNetwork, MockHistorySyncStream, BlockLiveSync<MockNetwork>>,
    MockNode<MockNetwork>,
    MockId<MockPeerId>,
) {
    let blockchain_proxy = BlockchainProxy::from(chain);
    let network = Arc::new(hub.new_network());

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy.clone(),
        Arc::clone(&network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy.clone(),
        Arc::clone(&network),
        block_queue,
        Arc::new(Mutex::new(BlsCache::new_test())),
    );

    let mut syncer = Syncer::new(
        blockchain_proxy,
        Arc::clone(&network),
        live_sync,
        MockHistorySyncStream::new(),
    );

    let mock_node =
        MockNode::with_network_and_blockchain(Arc::new(hub.new_network()), blockchain());
    network.dial_mock(&mock_node.network);
    let peer_id = mock_node.network.get_local_peer_id();
    syncer.live_sync.add_peer(peer_id);

    (syncer, mock_node, MockId::new(peer_id))
}

/// The proxy must push back once the consumer stops draining it. The consumer is never polled
/// here, which is what a slow blockchain push does for its duration: `LiveSyncer::poll_next` only
/// polls the proxy while it has no push operation in flight.
#[test(tokio::test)]
async fn block_queue_proxy_pushes_back_when_consumer_stalls() {
    let blockchain1 = blockchain();
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    // Small gossip channel, so backpressure becomes visible quickly.
    let (block_tx, block_rx) = mpsc::channel(4);
    let (_syncer, _mock_node, mock_id) = setup_live_sync(&mut hub, &blockchain1, block_rx);

    // All announcements are children of the current head, so they take the `parent_known` branch
    // and are emitted as `QueuedBlock::Head`. The head never moves, since the consumer never runs.
    let producer = BlockProducer::new(signing_key(), voting_key());
    let template = next_micro_block(&producer, &blockchain2);

    let mut accepted = 0;
    let mut pushed_back = false;

    for nonce in 0..MAX_ANNOUNCED_BLOCKS {
        let announcement = forge_variant(&template, nonce as u32);

        // Retry across scheduler rounds, so a full channel isn't just an unpolled background task.
        let mut delivered = false;
        for _ in 0..DRAIN_ROUNDS {
            match block_tx.try_send((announcement.clone(), mock_id.clone())) {
                Ok(()) => {
                    delivered = true;
                    break;
                }
                Err(TrySendError::Full(_)) => yield_now().await,
                Err(TrySendError::Closed(_)) => panic!("gossip stream closed unexpectedly"),
            }
        }

        if !delivered {
            pushed_back = true;
            break;
        }

        accepted += 1;
        yield_now().await;
    }

    assert!(
        pushed_back,
        "the proxy absorbed all {accepted} announced blocks without ever pushing back, so a peer \
         announcing faster than the consumer pushes can grow the proxy's channel without limit",
    );
}

/// Backpressure must not be implemented by dropping: a discarded `QueuedBlock::Head` is lost, as
/// nothing re-requests an announced block. Every announcement has to reach the consumer.
#[test(tokio::test)]
async fn block_queue_proxy_delivers_every_announced_block() {
    let blockchain1 = blockchain();
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let (block_tx, block_rx) = mpsc::channel(32);
    let (mut syncer, _mock_node, mock_id) = setup_live_sync(&mut hub, &blockchain1, block_rx);

    let producer = BlockProducer::new(signing_key(), voting_key());
    let template = next_micro_block(&producer, &blockchain2);

    let mut delivered: HashSet<Blake2bHash> = HashSet::new();
    let mut announced = 0;

    // Each announcement yields exactly one push and one event, so there is always something in
    // flight to await.
    while delivered.len() < NUM_DELIVERED_BLOCKS {
        // Hand over as much as the gossip stream currently accepts.
        while announced < NUM_DELIVERED_BLOCKS {
            let announcement = forge_variant(&template, announced as u32);
            match block_tx.try_send((announcement, mock_id.clone())) {
                Ok(()) => announced += 1,
                Err(TrySendError::Full(_)) => break,
                Err(TrySendError::Closed(_)) => panic!("gossip stream closed unexpectedly"),
            }
        }

        // Forged blocks fail verification, so every delivery surfaces as a rejection.
        yield_now().await;
        match syncer.next().await {
            Some(LiveSyncPushEvent::RejectedBlock(hash)) => {
                delivered.insert(hash);
            }
            Some(_) => {}
            None => panic!("syncer terminated before all blocks were delivered"),
        }
    }

    assert_eq!(
        delivered.len(),
        NUM_DELIVERED_BLOCKS,
        "every announced block must reach the consumer",
    );
}

#[test(tokio::test)]
async fn send_single_micro_block_to_block_queue() {
    let blockchain2 = blockchain();
    let blockchain_proxy = BlockchainProxy::from(&blockchain2);

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy.clone(),
        Arc::clone(&network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy.clone(),
        Arc::clone(&network),
        block_queue,
        Arc::new(Mutex::new(BlsCache::new_test())),
    );

    let mut syncer = Syncer::new(
        blockchain_proxy,
        Arc::clone(&network),
        live_sync,
        MockHistorySyncStream::new(),
    );

    let mock_node =
        MockNode::with_network_and_blockchain(Arc::new(hub.new_network()), blockchain());
    network.dial_mock(&mock_node.network);
    syncer
        .live_sync
        .add_peer(mock_node.network.get_local_peer_id());

    // Push one micro block to the queue
    let producer = BlockProducer::new(signing_key(), voting_key());
    let block = next_micro_block(&producer, &blockchain2);

    let mock_id = MockId::new(mock_node.network.get_local_peer_id());
    block_tx.send((block, mock_id)).await.unwrap();

    assert_eq!(
        blockchain2.read().block_number(),
        Policy::genesis_block_number()
    );

    // Run the block_queue one iteration, i.e. until it processed one block
    syncer.next().await;

    // The produced block is without gap and should go right into the blockchain
    assert_eq!(
        blockchain2.read().block_number(),
        1 + Policy::genesis_block_number()
    );
    assert_eq!(syncer.live_sync.queue().num_buffered_heights(), 0);
}

#[test(tokio::test)]
async fn send_two_micro_blocks_out_of_order() {
    let blockchain1 = blockchain();
    let blockchain_proxy_1 = BlockchainProxy::from(&blockchain1);
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());

    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        Arc::new(Mutex::new(BlsCache::new_test())),
    );

    let mut syncer = Syncer::new(
        blockchain_proxy_1,
        Arc::clone(&network),
        live_sync,
        MockHistorySyncStream::new(),
    );

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

    // Send block2 first
    block_tx
        .send((block2.clone(), mock_id.clone()))
        .await
        .unwrap();

    assert_eq!(
        blockchain1.read().block_number(),
        Policy::genesis_block_number()
    );

    // Run the block_queue one iteration, i.e. until it processed one block
    let _ = poll!(syncer.next());
    // Yield to allow the internal BlockQueue task to proceed.
    yield_now().await;

    // This block should be buffered now
    assert_eq!(
        blockchain1.read().block_number(),
        Policy::genesis_block_number()
    );
    let blocks = syncer.live_sync.queue().buffered_blocks();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.first().unwrap();
    assert_eq!(*block_number, 2 + Policy::genesis_block_number());
    assert_eq!(blocks[0], block2);

    // Also we should've received a request to fill this gap
    let req = mock_node.next().await.unwrap();
    assert_eq!(req, RequestMissingBlocks::TYPE_ID);

    // Now send block1 to fill the gap
    block_tx.send((block1.clone(), mock_id)).await.unwrap();

    // Run the block_queue until is has produced two events.
    syncer.next().await;
    syncer.next().await;

    // Now both blocks should've been pushed to the blockchain
    assert_eq!(
        blockchain1.read().block_number(),
        2 + Policy::genesis_block_number()
    );
    assert_eq!(syncer.live_sync.queue().num_buffered_heights(), 0);
    assert_eq!(
        blockchain1
            .read()
            .get_block_at(1 + Policy::genesis_block_number(), true, None)
            .unwrap(),
        block1
    );
    assert_eq!(
        blockchain1
            .read()
            .get_block_at(2 + Policy::genesis_block_number(), true, None)
            .unwrap(),
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
    let (block_tx, block_rx) = mpsc::channel(32);
    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );
    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        Arc::new(Mutex::new(BlsCache::new_test())),
    );
    let mut syncer = Syncer::new(
        blockchain_proxy_1,
        Arc::clone(&network),
        live_sync,
        MockHistorySyncStream::new(),
    );

    let mock_node =
        MockNode::with_network_and_blockchain(Arc::new(hub.new_network()), blockchain());
    network.dial_mock(&mock_node.network);
    syncer
        .live_sync
        .add_peer(mock_node.network.get_local_peer_id());

    let mut rng = test_rng(false);
    let mut ordered_blocks = Vec::new();

    let mock_id = MockId::new(mock_node.network.get_local_peer_id());
    let producer = BlockProducer::new(signing_key(), voting_key());
    let n_blocks = rng.random_range(2..15);

    for _ in 0..n_blocks {
        let block = push_micro_block(&producer, &blockchain2);
        ordered_blocks.push(block);
    }

    let mut blocks = ordered_blocks.clone();

    while blocks.len() > 1 {
        let index = rng.random_range(1..blocks.len());

        block_tx
            .send((blocks.remove(index).clone(), mock_id.clone()))
            .await
            .unwrap();

        // Run the block_queue one iteration, i.e. until it processed one block
        let _ = poll!(syncer.next());
        // Yield to allow the internal BlockQueue task to proceed.
        yield_now().await;
    }

    // All blocks should be buffered
    assert_eq!(
        blockchain1.read().block_number(),
        Policy::genesis_block_number()
    );

    // Obtain the buffered blocks
    assert_eq!(
        syncer.live_sync.queue().num_buffered_heights() as u64,
        n_blocks - 1
    );

    // Now send block1 to fill the gap
    block_tx.send((blocks[0].clone(), mock_id)).await.unwrap();

    for _ in 0..n_blocks {
        syncer.next().await;
    }

    // Verify all blocks except the genesis
    for i in 1..=n_blocks {
        assert_eq!(
            blockchain1
                .read()
                .get_block_at(i as u32 + Policy::genesis_block_number(), true, None)
                .unwrap(),
            ordered_blocks[(i - 1) as usize]
        );
    }

    // No blocks buffered
    assert_eq!(syncer.live_sync.queue().num_buffered_heights(), 0);
}

#[test(tokio::test)]
async fn send_invalid_block() {
    let blockchain1 = blockchain();
    let blockchain_proxy_1 = BlockchainProxy::from(&blockchain1);
    let blockchain2 = blockchain();

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        Arc::new(Mutex::new(BlsCache::new_test())),
    );

    let mut syncer = Syncer::new(
        blockchain_proxy_1,
        Arc::clone(&network),
        live_sync,
        MockHistorySyncStream::new(),
    );

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

    let mock_id = MockId::new(mock_node.network.get_local_peer_id());

    // Send block2 first
    block_tx
        .send((block2.clone(), mock_id.clone()))
        .await
        .unwrap();

    assert_eq!(
        blockchain1.read().block_number(),
        Policy::genesis_block_number()
    );

    // Run the block_queue one iteration, i.e. until it processed one block
    let _ = poll!(syncer.next());
    // Yield to allow the internal BlockQueue task to proceed.
    yield_now().await;

    // This block should be buffered now
    assert_eq!(
        blockchain1.read().block_number(),
        Policy::genesis_block_number()
    );
    let blocks = syncer.live_sync.queue().buffered_blocks();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.first().unwrap();
    assert_eq!(*block_number, 2 + Policy::genesis_block_number());
    assert_eq!(blocks[0], block2);

    let req = mock_node.next().await.unwrap();
    assert_eq!(req, RequestMissingBlocks::TYPE_ID);

    // Now send block1 to fill the gap
    block_tx.send((block1.clone(), mock_id)).await.unwrap();

    // Run the block_queue until is has produced two events.
    // The second block will be rejected due to an Invalid Successor event
    syncer.next().await;
    syncer.next().await;

    // Only Block 1 should be pushed to the blockchain
    assert_eq!(
        blockchain1.read().block_number(),
        1 + Policy::genesis_block_number()
    );
    assert_eq!(syncer.live_sync.queue().num_buffered_heights(), 0);
    assert_eq!(
        blockchain1
            .read()
            .get_block_at(1 + Policy::genesis_block_number(), true, None)
            .unwrap(),
        block1
    );
    assert_ne!(
        blockchain1
            .read()
            .get_block_at(1 + Policy::genesis_block_number(), true, None)
            .unwrap(),
        block2
    );
}

#[test(tokio::test)]
async fn send_block_with_gap_and_respond_to_missing_request() {
    let genesis_block_number = Policy::genesis_block_number();
    let blockchain1 = blockchain();
    let blockchain_proxy_1 = BlockchainProxy::from(&blockchain1);

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        Arc::new(Mutex::new(BlsCache::new_test())),
    );

    let mut syncer = Syncer::new(
        blockchain_proxy_1,
        Arc::clone(&network),
        live_sync,
        MockHistorySyncStream::new(),
    );

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

    // Send block2 first
    block_tx.send((block2.clone(), mock_id)).await.unwrap();

    assert_eq!(blockchain1.read().block_number(), genesis_block_number);

    // Run the block_queue one iteration, i.e. until it processed one block
    let _ = poll!(syncer.next());
    // Yield to allow the internal BlockQueue task to proceed.
    yield_now().await;

    // This block should be buffered now
    assert_eq!(blockchain1.read().block_number(), genesis_block_number);
    let blocks = syncer.live_sync.queue().buffered_blocks();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.first().unwrap();
    assert_eq!(*block_number, 2 + genesis_block_number);
    assert_eq!(blocks[0], block2);

    // Also we should've received a request to fill this gap
    // Instead of gossiping the block, we'll answer the missing blocks request
    let req = mock_node.next().await.unwrap();
    assert_eq!(req, RequestMissingBlocks::TYPE_ID);

    // Run the block_queue until is has produced two events.
    syncer.next().await;
    syncer.next().await;

    // Now both blocks should've been pushed to the blockchain
    assert_eq!(blockchain1.read().block_number(), 2 + genesis_block_number);
    assert_eq!(syncer.live_sync.queue().num_buffered_heights(), 0);
    assert_eq!(
        blockchain1
            .read()
            .get_block_at(1 + genesis_block_number, true, None)
            .unwrap(),
        block1
    );
    assert_eq!(
        blockchain1
            .read()
            .get_block_at(2 + genesis_block_number, true, None)
            .unwrap(),
        block2
    );
}

#[test(tokio::test)]
async fn request_missing_blocks_across_macro_block() {
    let genesis_block_number = Policy::genesis_block_number();
    let blockchain1 = blockchain();
    let blockchain_proxy_1 = BlockchainProxy::from(&blockchain1);

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        Arc::new(Mutex::new(BlsCache::new_test())),
    );

    let mut syncer = Syncer::new(
        blockchain_proxy_1,
        Arc::clone(&network),
        live_sync,
        MockHistorySyncStream::new(),
    );

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

    // Send block2 first
    block_tx.send((block2.clone(), mock_id)).await.unwrap();

    assert_eq!(blockchain1.read().block_number(), genesis_block_number);

    // Run the block_queue one iteration, i.e. until it processed one block
    let _ = poll!(syncer.next());
    // Yield to allow the internal BlockQueue task to proceed.
    yield_now().await;

    // This block should be buffered now
    assert_eq!(blockchain1.read().block_number(), genesis_block_number);
    let blocks = syncer.live_sync.queue().buffered_blocks();
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.first().unwrap();
    assert_eq!(*block_number, block2.block_number());
    assert_eq!(blocks[0], block2);

    // Also we should've received a request to fill the first gap.
    // Instead of gossiping the block, we'll answer the missing blocks request
    mock_node
        .request_missing_block_handler
        .set(|_mock_peer_id, req, blockchain| {
            assert_eq!(&req.target_hash, blockchain.read().head().parent_hash());
            let genesis = req.locators.first().unwrap();
            let blocks = blockchain
                .read()
                .get_blocks(
                    genesis,
                    Policy::blocks_per_batch(),
                    true,
                    Direction::Forward,
                )
                .unwrap();

            Ok(ResponseBlocks { blocks })
        });
    let req = mock_node.next().await.unwrap();
    mock_node.request_missing_block_handler.unset();
    assert_eq!(req, RequestMissingBlocks::TYPE_ID);

    // Run the block_queue one iteration, i.e. until it processed one batch.
    // Needs to be pulled again to send the request missing block over the network.
    syncer.next().await;

    let _ = poll!(syncer.next());
    // Yield to allow the internal BlockQueue task to proceed.
    yield_now().await;
    let _ = poll!(syncer.next());

    // The blocks from the first missing blocks request should be applied now
    assert_eq!(
        blockchain1.read().block_number(),
        Policy::blocks_per_batch() + genesis_block_number
    );
    let blocks = syncer.live_sync.queue().buffered_blocks();
    // We have the last block buffered.
    assert_eq!(blocks.len(), 1);
    let (block_number, blocks) = blocks.first().unwrap();
    assert_eq!(*block_number, block2.block_number());
    assert_eq!(blocks[0], block2);

    // Also we should've received a request to fill the second gap.
    let req = mock_node.next().await.unwrap();
    assert_eq!(req, RequestMissingBlocks::TYPE_ID);

    // Run the block_queue one iteration, i.e. until it processed the remaining blocks.
    syncer.next().await;
    assert_eq!(blockchain1.read().block_number(), block1.block_number());

    // Now make sure the last block from the buffer is also applied.
    syncer.next().await;

    // Now all blocks should've been pushed to the blockchain.
    assert_eq!(blockchain1.read().block_number(), block2.block_number());
    assert_eq!(syncer.live_sync.queue().num_buffered_heights(), 0);
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
    let history_sync = MockHistorySyncStream::new();
    let history_sync_peers = history_sync.peers.clone();
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig {
            buffer_max: 10,
            window_ahead_max: 10,
            tolerate_past_max: 100,
            include_body: true,
        },
    );

    let live_sync = BlockLiveSync::with_queue(
        blockchain_proxy_1.clone(),
        Arc::clone(&network),
        block_queue,
        Arc::new(Mutex::new(BlsCache::new_test())),
    );

    let mut syncer = Syncer::new(
        blockchain_proxy_1,
        Arc::clone(&network),
        live_sync,
        history_sync,
    );

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

    // Run the block_queue one iteration, i.e. until it processed one block
    let _ = poll!(syncer.next());

    // Yield execution to allow the internal BlockQueue task to proceed.
    yield_now().await;

    // Run the block queue another iteration.
    let _ = poll!(syncer.next());

    assert_eq!(history_sync_peers.read().len(), 1);
}
