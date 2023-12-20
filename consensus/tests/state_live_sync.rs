use std::{sync::Arc, task::Poll};

use futures::{join, poll, Future, Stream, StreamExt};
use log::info;
use nimiq_block::Block;
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::{
    messages::RequestMissingBlocks,
    sync::{
        live::{
            block_queue::BlockQueue,
            diff_queue::{DiffQueue, RequestTrieDiff, ResponseTrieDiff},
            queue::QueueConfig,
            state_queue::{Chunk, ChunkRequestState, RequestChunk, ResponseChunk, StateQueue},
            StateLiveSync,
        },
        syncer::{LiveSync, LiveSyncEvent, LiveSyncPeerEvent, LiveSyncPushEvent},
    },
};
use nimiq_database::{
    traits::{Database, WriteTransaction},
    volatile::VolatileDatabase,
};
use nimiq_genesis::{NetworkId, NetworkInfo};
use nimiq_hash::{Blake2bHash, Blake2sHash};
use nimiq_network_interface::{
    network::Network,
    request::{Handle, RequestCommon},
};
use nimiq_network_mock::{MockHub, MockId, MockNetwork, MockPeerId};
use nimiq_primitives::{key_nibbles::KeyNibbles, policy::Policy, trie::trie_diff::TrieDiff};
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::{produce_macro_blocks, push_micro_block, signing_key, voting_key},
    mock_node::MockNode,
    node::TESTING_BLS_CACHE_MAX_CAPACITY,
};
use nimiq_utils::{math::CeilingDiv, time::OffsetTime};
use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc::{self, Sender};
use tokio_stream::wrappers::ReceiverStream;

fn blockchain(complete: bool) -> Blockchain {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileDatabase::new(20).unwrap();
    let blockchain = Blockchain::new(
        env.clone(),
        BlockchainConfig::default(),
        NetworkId::UnitAlbatross,
        time,
    )
    .unwrap();

    if !complete {
        let mut txn = env.write_transaction();
        blockchain
            .state
            .accounts
            .reinitialize_as_incomplete(&mut (&mut txn).into());
        txn.commit();
    }

    blockchain
}

fn bls_cache() -> Arc<Mutex<PublicKeyCache>> {
    Arc::new(Mutex::new(PublicKeyCache::new(
        TESTING_BLS_CACHE_MAX_CAPACITY,
    )))
}

fn get_incomplete_live_sync(
    hub: &mut MockHub,
) -> (
    Arc<RwLock<Blockchain>>,
    StateLiveSync<MockNetwork>,
    Arc<MockNetwork>,
    Sender<(Block, MockId<MockPeerId>)>,
) {
    let incomplete_blockchain = Arc::new(RwLock::new(blockchain(false)));
    let incomplete_blockchain_proxy = BlockchainProxy::from(&incomplete_blockchain);

    let network = Arc::new(hub.new_network());
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        incomplete_blockchain_proxy.clone(),
        Arc::clone(&network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let diff_queue = DiffQueue::with_block_queue(Arc::clone(&network), block_queue);

    let state_queue = StateQueue::with_diff_queue(
        Arc::clone(&network),
        Arc::clone(&incomplete_blockchain),
        diff_queue,
        QueueConfig::default(),
    );

    let live_sync = StateLiveSync::with_queue(
        incomplete_blockchain_proxy,
        Arc::clone(&network),
        state_queue,
        bls_cache(),
    );

    (incomplete_blockchain, live_sync, network, block_tx)
}

async fn gossip_head_block(
    block_tx: &Sender<(Block, MockId<MockPeerId>)>,
    mock_id: MockId<MockPeerId>,
    blockchain: &Arc<RwLock<Blockchain>>,
) {
    let block = blockchain.read().state.main_chain.head.clone();
    block_tx.send((block, mock_id)).await.unwrap();
}

async fn test_chunk_reset<F, G, Fut>(pre_action: F, post_action: G, should_accept_block: bool)
where
    F: Fn(&mut MockNode<MockNetwork>),
    G: Fn(MockId<MockPeerId>, Sender<(Block, MockId<MockPeerId>)>, Arc<RwLock<Blockchain>>) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut hub = MockHub::new();

    // Setup the incomplete node.
    let (incomplete_blockchain, mut live_sync, network, block_tx) =
        get_incomplete_live_sync(&mut hub);

    // Setup the complete node.
    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();
    let genesis_accounts = network_info.genesis_accounts();
    let mut mock_node =
        MockNode::<MockNetwork>::new(2, genesis_block, genesis_accounts, &mut Some(hub)).await;

    // Connect the nodes.
    network.dial_mock(&mock_node.network);

    // Produce a couple of blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());
    let mock_id = MockId::new(mock_node.network.get_local_peer_id());

    push_micro_block(&producer, &mock_node.blockchain);
    gossip_head_block(&block_tx, mock_id.clone(), &mock_node.blockchain).await;

    // Sync state and blocks.
    let blockchain_rg = incomplete_blockchain.read();
    log::info!(
        "Incomplete blockchain: at #{} - {}, accounts: {:?}",
        blockchain_rg.block_number(),
        blockchain_rg.head_hash(),
        blockchain_rg
            .get_missing_accounts_range(None)
            .map(|v| v.start)
    );
    drop(blockchain_rg);

    live_sync.add_peer(mock_node.network.get_local_peer_id());

    // Will accept block.
    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestTrieDiff::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(
                    LiveSyncPushEvent::AcceptedAnnouncedBlock(_)
                )),
            )
        ),
        "Should accept announced block"
    );

    // Will request and apply chunks.
    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestChunk::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                    _
                ))),
            )
        ),
        "Should receive chunk"
    );
    // Check that we accepted the chunk.
    let first_chunk_missing_range = {
        let blockchain_rg = incomplete_blockchain.read();
        let missing_range = blockchain_rg.get_missing_accounts_range(None);
        assert!(missing_range.is_some());
        assert_ne!(missing_range, Some(KeyNibbles::ROOT..));
        missing_range.unwrap()
    };

    // Modify response.
    pre_action(&mut mock_node);

    let mock_node_request = {
        let mock_id = mock_id.clone();
        let block_tx = block_tx.clone();
        let blockchain = Arc::clone(&mock_node.blockchain);
        let mock_node = &mut mock_node;
        async move {
            let res = mock_node.next().await;
            post_action(mock_id, block_tx, blockchain).await;
            (res, mock_node.next().await)
        }
    };

    // Will request chunk.
    if should_accept_block {
        assert!(
            matches!(
                join!(mock_node_request, live_sync.next()),
                (
                    (Some(RequestChunk::TYPE_ID), Some(RequestTrieDiff::TYPE_ID)),
                    Some(LiveSyncEvent::PushEvent(
                        LiveSyncPushEvent::AcceptedAnnouncedBlock(_)
                    )),
                )
            ),
            "Should receive block"
        );
    } else {
        assert!(
            matches!(
                join!(mock_node_request, live_sync.next()),
                (
                    (Some(_), Some(_)),
                    Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::RejectedBlock(
                        _
                    ))),
                )
            ),
            "Should receive block"
        );
    }

    // Check that we reset the chain.
    // That means we will continue from the previous missing range.
    {
        let blockchain_rg = incomplete_blockchain.read();
        let missing_range = blockchain_rg.get_missing_accounts_range(None);
        assert_eq!(
            missing_range,
            Some(first_chunk_missing_range.clone()),
            "Should not have committed a chunk"
        );
    }
    // State can still be on `Reset` or if a request already happened it can be at `Continue` with the correct start.
    assert!(
        live_sync.queue().chunk_request_state()
            == &ChunkRequestState::Continue(first_chunk_missing_range.start)
            || live_sync.queue().chunk_request_state() == &ChunkRequestState::Reset,
        "Should have reset"
    );
}

async fn next<S: Stream + Unpin>(mut stream: S, n: usize) -> Option<Vec<S::Item>> {
    let mut result = Vec::new();
    for _ in 0..n {
        result.push(stream.next().await?);
    }
    Some(result)
}

#[test(tokio::test)]
async fn can_sync_state() {
    let mut hub = MockHub::new();

    // Setup the incomplete node.
    let (incomplete_blockchain, mut live_sync, network, block_tx) =
        get_incomplete_live_sync(&mut hub);

    // Setup the complete node.
    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();
    let genesis_accounts = network_info.genesis_accounts();
    let mut mock_node =
        MockNode::<MockNetwork>::new(2, genesis_block, genesis_accounts, &mut Some(hub)).await;
    let mock_id = MockId::new(mock_node.network.get_local_peer_id());

    // Connect the nodes.
    network.dial_mock(&mock_node.network);

    // Produce a couple of blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());
    push_micro_block(&producer, &mock_node.blockchain);
    gossip_head_block(&block_tx, mock_id.clone(), &mock_node.blockchain).await;

    // Sync state and blocks.
    let blockchain_rg = incomplete_blockchain.read();
    log::info!(
        "Incomplete blockchain: at #{} - {}, accounts: {:?}",
        blockchain_rg.block_number(),
        blockchain_rg.head_hash(),
        blockchain_rg
            .get_missing_accounts_range(None)
            .map(|v| v.start)
    );
    drop(blockchain_rg);
    live_sync.add_peer(mock_node.network.get_local_peer_id());

    // Will request chunks and receive the block.
    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestTrieDiff::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(
                    LiveSyncPushEvent::AcceptedAnnouncedBlock(_)
                )),
            )
        ),
        "Should immediately receive block"
    );
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 0);

    let size = mock_node.blockchain.read().state.accounts.size();
    let num_chunks = size.ceiling_div(Policy::state_chunks_max_size() as u64);
    for i in 0..num_chunks {
        info!("Applying chunk #{}", i);
        assert!(
            matches!(
                join!(mock_node.next(), live_sync.next()),
                (
                    Some(RequestChunk::TYPE_ID),
                    Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                        _
                    )))
                )
            ),
            "Should receive and accept chunks"
        );
        assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
        assert_eq!(live_sync.queue().num_buffered_blocks(), 0);

        let blockchain_rg = incomplete_blockchain.read();
        log::info!(
            "Incomplete blockchain: at #{} - {}, accounts: {:?}",
            blockchain_rg.block_number(),
            blockchain_rg.head_hash(),
            blockchain_rg
                .get_missing_accounts_range(None)
                .map(|v| v.start)
        );
        drop(blockchain_rg);
    }

    let blockchain_rg = incomplete_blockchain.read();
    assert!(blockchain_rg.accounts_complete());
    assert!(!live_sync.queue().chunk_request_state().is_complete());
    drop(blockchain_rg);

    produce_macro_blocks(&producer, &mock_node.blockchain, 1);
    gossip_head_block(&block_tx, mock_id.clone(), &mock_node.blockchain).await;

    // Will request missing blocks and apply those.
    assert!(
        matches!(
            join!(next(&mut mock_node, 31), live_sync.next()),
            (
                Some(_),
                Some(LiveSyncEvent::PushEvent(
                    LiveSyncPushEvent::ReceivedMissingBlocks(..)
                )),
            )
        ),
        "Should receive missing blocks"
    );
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 1);

    // Will apply the buffered block.
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(
                LiveSyncPushEvent::AcceptedBufferedBlock(..)
            ))
        ),
        "Should apply buffered block"
    );
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 0);

    push_micro_block(&producer, &mock_node.blockchain);
    gossip_head_block(&block_tx, mock_id.clone(), &mock_node.blockchain).await;

    // Will apply the announced block.
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(
                LiveSyncPushEvent::AcceptedAnnouncedBlock(..)
            ))
        ),
        "Should apply announced block"
    );
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 0);

    let blockchain_rg = incomplete_blockchain.read();
    assert!(blockchain_rg.accounts_complete());
    assert!(live_sync.queue().chunk_request_state().is_complete());
    drop(blockchain_rg);
}

#[test(tokio::test)]
async fn revert_chunks_for_state_live_sync() {
    let mut hub = MockHub::new();

    // Setup the incomplete node.
    let (incomplete_blockchain, mut live_sync, network, block_tx) =
        get_incomplete_live_sync(&mut hub);

    // Setup the complete node.
    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();
    let genesis_accounts = network_info.genesis_accounts();
    let mut mock_node =
        MockNode::<MockNetwork>::new(2, genesis_block, genesis_accounts, &mut Some(hub)).await;
    let mock_id = MockId::new(mock_node.network.get_local_peer_id());

    // Connect the nodes.
    network.dial_mock(&mock_node.network);

    // Produce a couple of blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());
    let producer2 = TemporaryBlockProducer::new();

    push_micro_block(&producer, &mock_node.blockchain);
    gossip_head_block(&block_tx, mock_id.clone(), &mock_node.blockchain).await;

    let fork1a = producer2.next_block(vec![0x48], false);
    let fork1b = producer2.next_block(vec![], false);

    // Sync state and blocks.
    let blockchain_rg = incomplete_blockchain.read();
    log::info!(
        "Incomplete blockchain: at #{} - {}, accounts: {:?}",
        blockchain_rg.block_number(),
        blockchain_rg.head_hash(),
        blockchain_rg
            .get_missing_accounts_range(None)
            .map(|v| v.start)
    );
    drop(blockchain_rg);
    live_sync.add_peer(mock_node.network.get_local_peer_id());

    // Will request a chunk and receive the block.
    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestTrieDiff::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(
                    LiveSyncPushEvent::AcceptedAnnouncedBlock(_)
                )),
            )
        ),
        "Should immediately receive block"
    );
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 0);

    info!("Applying chunk #{}", 0);
    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestChunk::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                    _
                )))
            )
        ),
        "Should receive and accept chunks"
    );
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 0);

    let blockchain_rg = incomplete_blockchain.read();
    log::info!(
        "Incomplete blockchain: at #{} - {}, accounts: {:?}",
        blockchain_rg.block_number(),
        blockchain_rg.head_hash(),
        blockchain_rg
            .get_missing_accounts_range(None)
            .map(|v| v.start)
    );
    drop(blockchain_rg);

    // Make a rebranch on the complete node
    assert_eq!(
        Blockchain::push(mock_node.blockchain.upgradable_read(), fork1a),
        Ok(PushResult::Forked),
    );
    assert_eq!(
        Blockchain::push(mock_node.blockchain.upgradable_read(), fork1b),
        Ok(PushResult::Rebranched),
    );

    info!("Requesting chunks #{}", 1);
    assert!(matches!(poll!(live_sync.next()), Poll::Pending));
    assert!(matches!(poll!(live_sync.next()), Poll::Pending));
    assert_eq!(mock_node.next().await, Some(RequestChunk::TYPE_ID));

    gossip_head_block(&block_tx, mock_id.clone(), &mock_node.blockchain).await;

    // Will request a chunk and receive the block.
    assert!(
        matches!(
            join!(next(&mut mock_node, 2), live_sync.next()),
            (
                Some(_),
                Some(LiveSyncEvent::PushEvent(
                    LiveSyncPushEvent::ReceivedMissingBlocks(..)
                )),
            )
        ),
        "Should immediately receive block"
    );
    assert_eq!(live_sync.queue().num_buffered_chunks(), 1);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 1);

    info!("Applying chunk #{}", 2);
    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestTrieDiff::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(
                    LiveSyncPushEvent::AcceptedBufferedBlock(..)
                )),
            )
        ),
        "Should receive and accept chunks"
    );
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 0);

    info!("Applying chunk #{}", 3);
    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestChunk::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                    ..
                )))
            )
        ),
        "Should receive and accept chunks"
    );

    info!("Applying chunk #{}", 4);
    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestChunk::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                    _
                )))
            )
        ),
        "Should receive and accept chunks"
    );

    // Checks that the state is complete but that the sync is still incomplete
    // and waiting for the macro block.
    let blockchain_rg = incomplete_blockchain.read();
    assert!(blockchain_rg.accounts_complete());
    assert!(!live_sync.queue().chunk_request_state().is_complete());
    assert_eq!(
        blockchain_rg.state.accounts.get_root_hash_assert(None),
        mock_node
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None),
        "Final accounts tries should be the same"
    );
    drop(blockchain_rg);
}

// Reset the chain of chunks
#[test(tokio::test)]
async fn can_reset_chain_of_chunks() {
    // Resets when:

    // all chunks ignored
    // Respond with to be ignored chunk and then produce and gossip a new block.
    test_chunk_reset(
        |mock_node| {
            mock_node
                .request_chunk_handler
                .set(|_mock_peer_id, _request, blockchain| {
                    let blockchain_rg = blockchain.read();
                    let chunk = blockchain_rg
                        .state
                        .accounts
                        .get_chunk(KeyNibbles::ROOT, 3, None);
                    ResponseChunk::Chunk(Chunk {
                        block_number: blockchain_rg.block_number(),
                        block_hash: blockchain_rg.head_hash(),
                        chunk,
                    })
                });
        },
        |mock_id, block_tx, blockchain| async move {
            let producer = BlockProducer::new(signing_key(), voting_key());

            push_micro_block(&producer, &blockchain);
            gossip_head_block(&block_tx, mock_id, &blockchain).await;
        },
        true,
    )
    .await;

    // error committing chunks
    // Respond with erroneous chunk and then produce and gossip a new block.
    test_chunk_reset(
        |mock_node| {
            mock_node
                .request_chunk_handler
                .set(|peer_id, request, blockchain| {
                    let mut chunk = <RequestChunk as Handle<
                        MockNetwork,
                        Arc<RwLock<Blockchain>>,
                    >>::handle(request, peer_id, blockchain);
                    // Make chunk invalid.
                    match chunk {
                        ResponseChunk::Chunk(ref mut inner_chunk) => {
                            inner_chunk.chunk.proof.nodes.pop();
                        }
                        _ => unreachable!(),
                    }
                    chunk
                });
        },
        |mock_id, block_tx, blockchain| async move {
            let producer = BlockProducer::new(signing_key(), voting_key());

            push_micro_block(&producer, &blockchain);
            gossip_head_block(&block_tx, mock_id, &blockchain).await;
        },
        true,
    )
    .await;

    // error block
    // Respond with good chunk for an unknown block and gossip the new block with error.
    test_chunk_reset(
        |mock_node| {
            let producer = BlockProducer::new(signing_key(), voting_key());
            push_micro_block(&producer, &mock_node.blockchain);

            mock_node
                .request_partial_diff_handler
                .set(|_, _, _| ResponseTrieDiff::PartialDiff(TrieDiff::default()));
        },
        |mock_id, block_tx, blockchain| async move {
            let mut block = blockchain.read().head();
            match block {
                Block::Micro(ref mut micro_block) => {
                    micro_block.header.body_root = Blake2sHash::default();
                }
                _ => unreachable!(),
            }
            block_tx.send((block, mock_id)).await.unwrap();
        },
        false,
    )
    .await;
}

// Remove chunks related to invalid blocks
#[test(tokio::test)]
async fn can_remove_chunks_related_to_invalid_blocks() {
    // Idea:
    // 1. Create the following blockchain
    // incomplete         mock
    //          |            |
    // [genesis] <- [1] <- [2]
    // 2. Give the incomplete node chunks for [2] and announce [2]
    // 3. Upon `request missing chunks`, return an invalid block
    // 4. Check that the chunks for [2] are removed
    // 5. Send valid chunks for the genesis block that are accepted immediately to make the live sync return
    let mut hub = MockHub::new();

    // Setup the incomplete node.
    let (_incomplete_blockchain, mut live_sync, network, block_tx) =
        get_incomplete_live_sync(&mut hub);

    // Setup the complete node.
    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();
    let genesis_accounts = network_info.genesis_accounts();
    let mut mock_node =
        MockNode::<MockNetwork>::new(2, genesis_block, genesis_accounts, &mut Some(hub)).await;
    let mock_id = MockId::new(mock_node.network.get_local_peer_id());

    // Connect the nodes.
    network.dial_mock(&mock_node.network);

    // Produce a couple of blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());

    push_micro_block(&producer, &mock_node.blockchain); // block [1]
    push_micro_block(&producer, &mock_node.blockchain); // block [2]

    live_sync.add_peer(mock_node.network.get_local_peer_id());

    // Return invalid missing block instead.
    mock_node
        .request_missing_block_handler
        .set(|mock_id, request, blockchain_proxy| {
            let mut response =
                <RequestMissingBlocks as Handle<MockNetwork, BlockchainProxy>>::handle(
                    request,
                    mock_id,
                    blockchain_proxy,
                );
            match response.blocks {
                Some(ref mut blocks) => match blocks[0] {
                    Block::Micro(ref mut micro_block) => {
                        micro_block.body = None;
                    }
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            }
            response
        });

    mock_node
        .request_partial_diff_handler
        .set(|_, _, _| ResponseTrieDiff::PartialDiff(TrieDiff::default()));

    let mock_node_fut = async move {
        let res1 = mock_node.next().await;
        gossip_head_block(&block_tx, mock_id.clone(), &mock_node.blockchain).await;
        let res2 = mock_node.next().await;

        // Revert to the genesis block.
        let new_blockchain = blockchain(true);
        {
            let mut blockchain_wg = mock_node.blockchain.write();
            *blockchain_wg = new_blockchain;
        }

        assert_eq!(mock_node.next().await, Some(RequestTrieDiff::TYPE_ID));
        assert_eq!(mock_node.next().await, Some(RequestChunk::TYPE_ID));
        assert_eq!(mock_node.next().await, Some(RequestChunk::TYPE_ID));
        (res1, res2)
    };

    // The live sync initially will not return since we did not accept a block.
    // We then send a chunk for the genesis block, which should be accepted.
    assert!(
        matches!(
            join!(mock_node_fut, live_sync.next()),
            (
                (
                    Some(RequestChunk::TYPE_ID),
                    Some(RequestMissingBlocks::TYPE_ID)
                ),
                Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                    _
                )))
            )
        ),
        "Should receive and accept chunks after reset"
    );

    // Check buffer cleared.
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 0);
}

// Buffer clearing after macro blocks
#[test(tokio::test)]
async fn clears_buffer_after_macro_block() {
    // Idea:
    // 1. Send a chunk for some block that does not exist.
    // 2. We produce a bunch of blocks and sync up to the macro block.
    // 3. Check that the buffered chunk disappears.
    let mut hub = MockHub::new();

    // Setup the incomplete node.
    let (_incomplete_blockchain, mut live_sync, network, block_tx) =
        get_incomplete_live_sync(&mut hub);

    // Setup the complete node.
    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();
    let genesis_accounts = network_info.genesis_accounts();
    let mut mock_node =
        MockNode::<MockNetwork>::new(2, genesis_block, genesis_accounts, &mut Some(hub)).await;
    let mock_id = MockId::new(mock_node.network.get_local_peer_id());

    // Connect the nodes.
    network.dial_mock(&mock_node.network);

    // Produce a couple of blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());

    live_sync.add_peer(mock_node.network.get_local_peer_id());

    // Push first block.
    push_micro_block(&producer, &mock_node.blockchain); // block [1]
    gossip_head_block(&block_tx, mock_id.clone(), &mock_node.blockchain).await;

    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestTrieDiff::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(
                    LiveSyncPushEvent::AcceptedAnnouncedBlock(_)
                )),
            )
        ),
        "Should accept first block"
    );

    // Upon first chunk request, return a chunk for a non-existent block.
    mock_node
        .request_chunk_handler
        .set(|mock_id, request, blockchain| {
            let mut chunk = <RequestChunk as nimiq_network_interface::request::Handle<
                MockNetwork,
                Arc<RwLock<Blockchain>>,
            >>::handle(request, mock_id, blockchain);
            match chunk {
                ResponseChunk::Chunk(ref mut inner_chunk) => {
                    inner_chunk.block_hash = Blake2bHash::default();
                }
                _ => unreachable!(),
            }
            chunk
        });

    let mock_node_fut = async move {
        assert_eq!(mock_node.next().await, Some(RequestChunk::TYPE_ID));
        mock_node.request_chunk_handler.unset();
        assert_eq!(mock_node.next().await, Some(RequestChunk::TYPE_ID));
        assert_eq!(mock_node.next().await, Some(RequestChunk::TYPE_ID));
        mock_node
    };

    let (mut mock_node, live_sync_result) = join!(mock_node_fut, live_sync.next());
    assert!(
        matches!(
            live_sync_result,
            Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                _
            )))
        ),
        "Should receive and accept chunks after reset"
    );

    // Check buffer.
    assert_eq!(live_sync.queue().num_buffered_chunks(), 1);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 0);

    produce_macro_blocks(&producer, &mock_node.blockchain, 1);
    gossip_head_block(&block_tx, mock_id.clone(), &mock_node.blockchain).await;

    mock_node.request_chunk_handler.pause();

    // Apply missing blocks.
    assert!(
        matches!(
            join!(next(&mut mock_node, 31), live_sync.next()),
            (
                Some(_),
                Some(LiveSyncEvent::PushEvent(
                    LiveSyncPushEvent::ReceivedMissingBlocks(..)
                ))
            )
        ),
        "Should accept missing blocks"
    );

    // Check buffer.
    assert_eq!(live_sync.queue().num_buffered_chunks(), 1);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 1);

    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestTrieDiff::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(
                    LiveSyncPushEvent::AcceptedBufferedBlock(..)
                )),
            )
        ),
        "Should accept missing blocks"
    );

    mock_node.request_chunk_handler.unpause();

    assert!(
        matches!(
            join!(mock_node.next(), live_sync.next()),
            (
                Some(RequestChunk::TYPE_ID),
                Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                    ..
                ))),
            )
        ),
        "Should accept chunks"
    );

    // Check buffer.
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_blocks(), 0);
}

// Check correct reply for incomplete nodes
#[test(tokio::test)]
async fn replies_with_incomplete_response_chunk() {
    let mut hub = MockHub::new();

    // Setup the incomplete node.
    let (_incomplete_blockchain, mut live_sync, network, _block_tx) =
        get_incomplete_live_sync(&mut hub);

    // Setup the complete node.
    let mut mock_node = MockNode::<MockNetwork>::with_network_and_blockchain(
        Arc::new(hub.new_network()),
        Arc::new(RwLock::new(blockchain(false))),
    );

    // Connect the nodes.
    network.dial_mock(&mock_node.network);
    live_sync.add_peer(mock_node.network.get_local_peer_id());

    assert!(
        matches!(
            join!(live_sync.next(), mock_node.next()),
            (
                Some(LiveSyncEvent::PeerEvent(LiveSyncPeerEvent::Behind(_))),
                Some(RequestChunk::TYPE_ID)
            )
        ),
        "Should not receive chunks from a peer with incomplete state"
    );
}
