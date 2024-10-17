use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{poll, Stream, StreamExt};
use nimiq_block::Block;
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, Direction};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::{
    messages::{RequestMissingBlocks, ResponseBlocks},
    sync::{
        live::{block_queue::BlockQueue, queue::QueueConfig, BlockLiveSync},
        syncer::{LiveSync, MacroSync, MacroSyncReturn, Syncer},
    },
};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_network_interface::{network::Network, request::RequestCommon};
use nimiq_network_mock::{MockHub, MockId, MockPeerId};
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{
        next_micro_block, produce_macro_blocks, push_micro_block, signing_key, voting_key,
    },
    mock_node::MockNode,
    node::TESTING_BLS_CACHE_MAX_CAPACITY,
};
use nimiq_utils::time::OffsetTime;
use parking_lot::{Mutex, RwLock};
use tokio::{sync::mpsc, task::yield_now};
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
        bls_cache(),
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
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks(), 0);
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
        bls_cache(),
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
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks(), 0);
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
async fn try_to_induce_request_missing_blocks_gaps() {
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
        bls_cache(),
    );

    let mut syncer = Syncer::new(
        blockchain_proxy_1,
        Arc::clone(&network),
        live_sync,
        MockHistorySyncStream::new(),
    );

    let mut mock_node = MockNode::with_network_and_blockchain(
        Arc::new(hub.new_network()),
        Arc::clone(&blockchain2),
    );

    network.dial_mock(&mock_node.network);
    syncer
        .live_sync
        .add_peer(mock_node.network.get_local_peer_id());

    let mut blocks = Vec::new();

    let mock_id = MockId::new(mock_node.network.get_local_peer_id());
    let producer = BlockProducer::new(signing_key(), voting_key());
    let total_blocks: usize = 10;
    let first_gossiped_block: usize = 6;

    for _ in 0..total_blocks {
        let block = push_micro_block(&producer, &blockchain2);
        blocks.push(block);
    }

    // Send the 6th block.
    block_tx
        .send((blocks[first_gossiped_block - 1].clone(), mock_id.clone()))
        .await
        .unwrap();
    // Run the block_queue one iteration, i.e. until it processed one block.
    let _ = poll!(syncer.next());
    // Yield to allow the internal BlockQueue task to proceed.
    yield_now().await;

    // Only block 6 should be buffered and a request missing blocks should be sent.
    assert_eq!(
        blockchain1.read().block_number(),
        Policy::genesis_block_number()
    );
    // Obtain the buffered blocks
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks() as u64, 1);

    // Send block 8, thus creating a gap and assert that nothing was done with it.
    let index = 7;
    block_tx
        .send((blocks[index].clone(), mock_id.clone()))
        .await
        .unwrap();
    let _ = poll!(syncer.next());
    yield_now().await;

    // No new blocks should be buffered
    assert_eq!(
        blockchain1.read().block_number(),
        Policy::genesis_block_number()
    );
    // We should have 1 buffered block, the first block that was gossiped.
    // We discard the new blocks with gaps until the chain fills to block 6.
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks() as u64, 1);

    // Now send blocks 1-5 to fill the gap.
    for i in 0..first_gossiped_block - 1 {
        block_tx
            .send((blocks[i].clone(), mock_id.clone()))
            .await
            .unwrap();
        syncer.next().await;
    }
    syncer.next().await;

    // Verify all blocks except the genesis.
    for i in 1..=first_gossiped_block - 1 {
        assert_eq!(
            blockchain1
                .read()
                .get_block_at(i as u32 + Policy::genesis_block_number(), true, None)
                .unwrap(),
            blocks[(i - 1) as usize]
        );
    }
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks(), 0);

    // Send block 7.
    // It should still not work because we have a pending missing blocks request.
    let index = 8;
    block_tx
        .send((blocks[index].clone(), mock_id.clone()))
        .await
        .unwrap();
    let _ = poll!(syncer.next());
    yield_now().await;
    assert_eq!(
        blockchain1.read().block_number(),
        first_gossiped_block as u32 + Policy::genesis_block_number()
    );
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks() as u64, 0);

    // Let old missing blocks request be processed.
    // The reply will result in the blocks being discarded since they have been applied through the block stream.
    _ = poll!(mock_node.next());
    syncer.next().await;

    // Send block 7 once more. Now it should be buffered and a missing blocks request should be sent.
    let index = 8;
    block_tx
        .send((blocks[index].clone(), mock_id.clone()))
        .await
        .unwrap();
    let _ = poll!(syncer.next());
    yield_now().await;
    assert_eq!(
        blockchain1.read().block_number(),
        first_gossiped_block as u32 + Policy::genesis_block_number()
    );
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks() as u64, 1);

    // Allow the last requests missing blocks to finish.
    _ = poll!(mock_node.next());
    syncer.next().await;
    syncer.next().await; // To push the buffered block

    assert_eq!(blockchain1.read().head_hash(), blocks[index].hash());
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks() as u64, 0);
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
        bls_cache(),
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
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks(), 0);
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
        bls_cache(),
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
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks(), 0);
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
        bls_cache(),
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
    assert_eq!(syncer.live_sync.queue().num_buffered_blocks(), 0);
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
        bls_cache(),
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
