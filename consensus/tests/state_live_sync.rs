use std::{
    str::FromStr,
    sync::{atomic::AtomicU32, Arc},
    task::Poll,
    time::Duration,
};

use futures::{join, poll, Future, Stream, StreamExt};
use log::info;
use nimiq_block::Block;
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{
    AbstractBlockchain, BlockchainEvent, ChunksPushResult, PushResult,
};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::{
    messages::RequestMissingBlocks,
    sync::{
        live::{
            block_queue::{BlockQueue, BlockSource},
            diff_queue::{
                diff_request_component::{DiffRequestComponent, DiffRequestError},
                DiffQueue, RequestTrieDiff, ResponseTrieDiff,
            },
            queue::{ChunkAndSource, LiveSyncQueue, QueueConfig},
            state_queue::{
                live_sync::PushOpResult, Chunk, ChunkRequestState, QueuedStateChunks, RequestChunk,
                ResponseChunk, StateQueue,
            },
            StateLiveSync,
        },
        peer_list::PeerList,
        sync_interface::{LiveSync, LiveSyncEvent, LiveSyncPeerEvent, LiveSyncPushEvent},
    },
    BlsCache,
};
use nimiq_database::{
    mdbx::MdbxDatabase,
    traits::{Database, WriteTransaction},
};
use nimiq_genesis::{NetworkId, NetworkInfo};
use nimiq_hash::{Blake2bHash, Blake2sHash};
use nimiq_keys::{Address, KeyPair, PrivateKey};
use nimiq_network_interface::{
    network::Network,
    request::{Handle, RequestCommon},
};
use nimiq_network_mock::{MockHub, MockId, MockNetwork, MockPeerId};
use nimiq_primitives::{
    coin::Coin, key_nibbles::KeyNibbles, policy::Policy, trie::trie_diff::TrieDiff,
};
use nimiq_test_log::test;
use nimiq_test_utils::{
    block_production::TemporaryBlockProducer,
    blockchain::{
        generate_transactions, produce_macro_blocks, push_micro_block, signing_key, voting_key,
        REWARD_KEY,
    },
    mock_node::MockNode,
};
use nimiq_time::timeout;
use nimiq_transaction::ExecutedTransaction;
use nimiq_transaction_builder::TransactionBuilder;
use nimiq_utils::{spawn, time::OffsetTime};
use parking_lot::{Mutex, RwLock};
use tokio::{
    sync::mpsc::{self, Sender},
    task::yield_now,
};
use tokio_stream::wrappers::ReceiverStream;

fn blockchain(complete: bool) -> Blockchain {
    let time = Arc::new(OffsetTime::new());
    let env = MdbxDatabase::new_volatile(Default::default()).unwrap();
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

fn get_incomplete_live_sync(
    hub: &mut MockHub,
) -> (
    Arc<RwLock<Blockchain>>,
    StateLiveSync<MockNetwork>,
    Arc<MockNetwork>,
    Sender<(Block, MockId<MockPeerId>)>,
) {
    let incomplete_blockchain = Arc::new(RwLock::new(blockchain(false)));
    let network = Arc::new(hub.new_network());
    let (live_sync, block_tx) = live_sync_over(&incomplete_blockchain, &network);
    (incomplete_blockchain, live_sync, network, block_tx)
}

/// Builds a state live sync over an existing blockchain, as a restarting node would.
fn live_sync_over(
    blockchain: &Arc<RwLock<Blockchain>>,
    network: &Arc<MockNetwork>,
) -> (
    StateLiveSync<MockNetwork>,
    Sender<(Block, MockId<MockPeerId>)>,
) {
    live_sync_over_with_progress(blockchain, network, Arc::new(AtomicU32::new(0)))
}

/// Like `live_sync_over`, but reports the state sync progress into `sync_progress`.
fn live_sync_over_with_progress(
    blockchain: &Arc<RwLock<Blockchain>>,
    network: &Arc<MockNetwork>,
    sync_progress: Arc<AtomicU32>,
) -> (
    StateLiveSync<MockNetwork>,
    Sender<(Block, MockId<MockPeerId>)>,
) {
    let blockchain_proxy = BlockchainProxy::from(blockchain);
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy.clone(),
        Arc::clone(network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let diff_queue = DiffQueue::with_block_queue(Arc::clone(network), block_queue);

    let state_queue = StateQueue::with_diff_queue(
        Arc::clone(network),
        Arc::clone(blockchain),
        diff_queue,
        QueueConfig::default(),
        sync_progress,
    );

    let live_sync = StateLiveSync::with_queue(
        blockchain_proxy,
        Arc::clone(network),
        state_queue,
        Arc::new(Mutex::new(BlsCache::new_test())),
    );

    (live_sync, block_tx)
}

fn get_state_queue(
    network: Arc<MockNetwork>,
    blockchain: Arc<RwLock<Blockchain>>,
) -> (StateQueue<MockNetwork>, Sender<(Block, MockId<MockPeerId>)>) {
    let blockchain_proxy = BlockchainProxy::from(&blockchain);
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        blockchain_proxy,
        Arc::clone(&network),
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );
    let diff_queue = DiffQueue::with_block_queue(Arc::clone(&network), block_queue);
    let state_queue = StateQueue::with_diff_queue(
        network,
        blockchain,
        diff_queue,
        QueueConfig::default(),
        Arc::new(AtomicU32::new(0)),
    );

    (state_queue, block_tx)
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
    yield_now().await;

    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);

    let size = mock_node.blockchain.read().state.accounts.size();
    let num_chunks = size.div_ceil(Policy::state_chunks_max_size() as u64);
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
        yield_now().await;

        assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
        assert_eq!(live_sync.queue().num_buffered_heights(), 0);

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
    yield_now().await;

    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);

    // Will request the diff for the buffered macro block and apply it.
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
        "Should apply buffered block"
    );
    yield_now().await;

    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);

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
    yield_now().await;

    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);

    let blockchain_rg = incomplete_blockchain.read();
    assert!(blockchain_rg.accounts_complete());
    assert!(live_sync.queue().chunk_request_state().is_complete());
    drop(blockchain_rg);
}

#[test(tokio::test)]
async fn gives_up_diff_request_if_only_source_knows_block_and_never_serves_diff() {
    let mut hub = Some(MockHub::new());
    let network = Arc::new(hub.as_mut().unwrap().new_network());

    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();
    let genesis_accounts = network_info.genesis_accounts();

    let mut source_node =
        MockNode::<MockNetwork>::new(2, genesis_block.clone(), genesis_accounts.clone(), &mut hub)
            .await;
    let mut other_node =
        MockNode::<MockNetwork>::new(3, genesis_block, genesis_accounts, &mut hub).await;

    network.dial_mock(&source_node.network);
    network.dial_mock(&other_node.network);

    // Only the source node has the block whose diff we are going to request.
    let producer = BlockProducer::new(signing_key(), voting_key());
    push_micro_block(&producer, &source_node.blockchain);
    let block = source_node.blockchain.read().state.main_chain.head.clone();

    // The source peer knows the block but never serves a usable diff for it.
    source_node
        .request_partial_diff_handler
        .set(|_, _, _| ResponseTrieDiff::IncompleteState);

    let peers = Arc::new(RwLock::new(PeerList::default()));
    // Insert a different peer first to prove that the request logic still prioritizes
    // the original sender before falling back to the rest of the peer set.
    peers
        .write()
        .add_peer(other_node.network.get_local_peer_id());
    peers
        .write()
        .add_peer(source_node.network.get_local_peer_id());

    let mut diff_request_component = DiffRequestComponent::new(Arc::clone(&network), peers);
    let mut get_diff = diff_request_component.request_diff();
    let source_peer_id = source_node.network.get_local_peer_id();
    // Drive the request in the background so we can observe which peers it contacts.
    let diff_request =
        tokio::spawn(
            async move { get_diff(&(block, BlockSource::requested(source_peer_id))).await },
        );

    // The source peer is asked first.
    assert_eq!(source_node.next().await, Some(RequestTrieDiff::TYPE_ID));
    // Once that fails, the requester falls back to the next peer, which does not know the block.
    assert_eq!(other_node.next().await, Some(RequestTrieDiff::TYPE_ID));
    // After all peers fail to provide a valid diff, the request gives up instead of starting another pass.
    assert!(
        matches!(
            diff_request.await.unwrap(),
            Err(DiffRequestError::MaxTriesExceeded)
        ),
        "diff request should fail after exhausting the peer set"
    );
    // No new request should be sent to the source peer after the request gives up.
    assert!(
        timeout(Duration::from_millis(100), source_node.next())
            .await
            .is_err(),
        "diff request should not retry once the maximum number of tries is reached"
    );
}

#[test(tokio::test)]
async fn buffered_diffless_block_is_retried_after_state_reset() {
    let mut hub = Some(MockHub::new());
    let (target, mut live_sync, network, block_tx) =
        get_incomplete_live_sync(hub.as_mut().unwrap());

    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block();
    let genesis_accounts = network_info.genesis_accounts();
    let mut source =
        MockNode::<MockNetwork>::new(2, genesis_block, genesis_accounts, &mut hub).await;
    let source_id = source.network.get_local_peer_id();
    let source_mock_id = MockId::new(source_id);

    network.dial_mock(&source.network);
    live_sync.add_peer(source_id);

    // Hold a diff request at the front of the ordered queue while the account state finishes
    // syncing, so the buffered child can remain queued behind it once released.
    source.request_partial_diff_handler.pause();
    let blocker = TemporaryBlockProducer::new().next_block(vec![0x42], false);
    let blocker_hash = blocker.hash();
    block_tx
        .send((blocker, source_mock_id.clone()))
        .await
        .unwrap();
    yield_now().await;
    assert!(matches!(poll!(live_sync.next()), Poll::Pending));

    let size = source.blockchain.read().state.accounts.size();
    let num_chunks = size.div_ceil(Policy::state_chunks_max_size() as u64);
    for _ in 0..num_chunks {
        let (request, event) = timeout(Duration::from_secs(5), async {
            join!(source.next(), live_sync.next())
        })
        .await
        .expect("state chunks should continue while the front diff request is pending");
        assert_eq!(request, Some(RequestChunk::TYPE_ID));
        assert!(matches!(
            event,
            Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                _
            )))
        ));
    }
    assert!(target.read().accounts_complete());

    let producer = BlockProducer::new(signing_key(), voting_key());
    push_micro_block(&producer, &source.blockchain);
    let applied_parent = source.blockchain.read().head().clone();
    let applied_parent_hash = applied_parent.hash();
    push_micro_block(&producer, &source.blockchain);
    let retried_block = source.blockchain.read().head().clone();
    let retried_hash = retried_block.hash();
    assert_ne!(blocker_hash, retried_hash);
    assert_eq!(retried_block.parent_hash(), &applied_parent_hash);

    // Announce the child first so BlockQueue buffers it until the parent is applied.
    source.request_missing_block_handler.pause();
    block_tx
        .send((retried_block, source_mock_id))
        .await
        .unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            yield_now().await;
            assert!(matches!(poll!(live_sync.next()), Poll::Pending));
            if live_sync.queue().num_buffered_heights() == 1 {
                break;
            }
        }
    })
    .await
    .expect("the child should be buffered while its parent is unknown");

    // Applying the parent emits the real `Extended` event that marks state sync complete and
    // releases the child as `QueuedBlock::Buffered`. Yield to BlockQueue's proxy task and then
    // poll live sync before checking the raw buffer count; this guarantees DiffQueue classifies
    // the child as diffless before the state reset.
    assert_eq!(
        Blockchain::push(target.upgradable_read(), applied_parent),
        Ok(PushResult::Extended)
    );
    timeout(Duration::from_secs(5), async {
        loop {
            yield_now().await;
            assert!(matches!(poll!(live_sync.next()), Poll::Pending));
            if live_sync.queue().num_buffered_heights() == 0 {
                break;
            }
        }
    })
    .await
    .expect("the buffered child should enter DiffQueue before state is reset");
    assert!(matches!(poll!(live_sync.next()), Poll::Pending));
    assert_eq!(target.read().head_hash(), applied_parent_hash);
    assert!(live_sync.queue().chunk_request_state().is_complete());

    // Notify the live-sync event handler about the reset, then let the new chunk response pair
    // with the stale diffless block that is still waiting behind the blocker.
    {
        let blockchain = target.read();
        let mut txn = blockchain.write_transaction();
        blockchain
            .state
            .accounts
            .reinitialize_as_incomplete(&mut (&mut txn).into());
        txn.commit();
        blockchain
            .notifier
            .send(BlockchainEvent::Rebranched(vec![], vec![]))
            .unwrap();
    }
    assert!(matches!(poll!(live_sync.next()), Poll::Pending));
    assert!(matches!(poll!(live_sync.next()), Poll::Pending));
    assert_eq!(
        target
            .read()
            .get_missing_accounts_range(None)
            .map(|range| range.start),
        Some(KeyNibbles::ROOT)
    );

    let expected_missing_start = source
        .blockchain
        .read()
        .state
        .accounts
        .get_chunk(
            KeyNibbles::ROOT,
            Policy::state_chunks_max_size() as usize,
            None,
        )
        .end_key
        .expect("the fixture should need more than one state chunk");
    assert_eq!(
        timeout(Duration::from_secs(5), source.next())
            .await
            .expect("state reset should request a fresh chunk"),
        Some(RequestChunk::TYPE_ID)
    );
    source.request_chunk_handler.pause();
    assert!(matches!(poll!(live_sync.next()), Poll::Pending));
    assert_eq!(live_sync.queue().num_buffered_chunks(), 1);

    // Queue a new diff request after the reset and leave it unresolved. It sits behind the stale
    // no-diff item, but must not remain ahead of that item's retry.
    let competing_network = Arc::new(hub.as_mut().unwrap().new_network_with_address(3_u64));
    let mut competing_diff_requests = competing_network.receive_requests::<RequestTrieDiff>();
    let _competing_chunk_requests = competing_network.receive_requests::<RequestChunk>();
    network.dial_mock(&competing_network);
    let competing_id = competing_network.get_local_peer_id();
    live_sync.add_peer(competing_id);

    let competing_block = TemporaryBlockProducer::new().next_block(vec![0x48], false);
    let competing_hash = competing_block.hash();
    block_tx
        .send((competing_block, MockId::new(competing_id)))
        .await
        .unwrap();
    let (competing_request, _competing_request_id, _) = timeout(Duration::from_secs(5), async {
        loop {
            if let Poll::Ready(request) = poll!(competing_diff_requests.next()) {
                break request.expect("competing diff request stream should stay open");
            }
            assert!(matches!(poll!(live_sync.next()), Poll::Pending));
            yield_now().await;
        }
    })
    .await
    .expect("post-reset competing diff request should be dispatched");
    assert_eq!(competing_request.block_hash, competing_hash);

    // Drop the original blocker across both peers, then serve the real diff for the stale block.
    // With `push_back`, the unresolved competing request remains ahead and this operation times out.
    source
        .request_partial_diff_handler
        .set(|_, _, _| ResponseTrieDiff::IncompleteState);
    source.request_partial_diff_handler.unpause();
    let source_requests = async {
        let blocker_request = source.next().await;
        source.request_partial_diff_handler.unset();
        let retry_request = source.next().await;
        (blocker_request, retry_request)
    };
    let competing_blocker_response = async {
        let (request, request_id, _) = competing_diff_requests
            .next()
            .await
            .expect("blocker should fall back to the competing peer");
        assert_eq!(request.block_hash, blocker_hash);
        competing_network
            .respond::<RequestTrieDiff>(request_id, ResponseTrieDiff::IncompleteState)
            .await
            .unwrap();
        RequestTrieDiff::TYPE_ID
    };
    let ((blocker_request, retry_request), fallback_request, event) =
        timeout(Duration::from_secs(3), async {
            join!(
                source_requests,
                competing_blocker_response,
                live_sync.next()
            )
        })
        .await
        .expect("retried block should be prioritized over the unresolved diff request");

    assert_eq!(blocker_request, Some(RequestTrieDiff::TYPE_ID));
    assert_eq!(retry_request, Some(RequestTrieDiff::TYPE_ID));
    assert_eq!(fallback_request, RequestTrieDiff::TYPE_ID);
    assert!(matches!(
        event,
        Some(LiveSyncEvent::PushEvent(
            LiveSyncPushEvent::AcceptedBufferedBlock(hash, 0)
        )) if hash == retried_hash
    ));

    let target = target.read();
    assert_eq!(target.head_hash(), retried_hash);
    assert!(target.contains(&applied_parent_hash, true));
    assert!(target.contains(&retried_hash, true));
    assert_eq!(
        target
            .get_missing_accounts_range(None)
            .map(|range| range.start),
        Some(expected_missing_start)
    );
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
}

#[test(tokio::test)]
async fn missing_needs_diff_reports_prefix_and_retries_paired_suffix() {
    let source_producer = TemporaryBlockProducer::new();
    let target_producer = TemporaryBlockProducer::new_incomplete();

    let first_block = source_producer.next_block(vec![], false);
    let first_hash = first_block.hash();
    let first_diff = source_producer
        .blockchain
        .read()
        .chain_store
        .get_accounts_diff(&first_hash, None)
        .unwrap();
    let first_retry_block = source_producer.next_block(vec![0x01], false);
    let first_retry_hash = first_retry_block.hash();
    let second_retry_block = source_producer.next_block(vec![0x02], false);
    let second_retry_hash = second_retry_block.hash();
    // These blocks have no transactions, so their accounts roots are identical and the chunks
    // read at the final suffix block are also valid for the first suffix block.
    let first_retry_chunk =
        source_producer.get_chunk(KeyNibbles::ROOT, Policy::state_chunks_max_size() as usize);
    let second_retry_start = first_retry_chunk
        .chunk
        .end_key
        .clone()
        .expect("the fixture should need exactly two state chunks");
    let second_retry_chunk = source_producer.get_chunk(
        second_retry_start.clone(),
        Policy::state_chunks_max_size() as usize,
    );
    assert!(second_retry_chunk.chunk.end_key.is_none());
    assert_eq!(second_retry_chunk.start_key, second_retry_start);

    let mut hub = MockHub::new();
    let network = Arc::new(hub.new_network());
    let source_network = Arc::new(hub.new_network());
    let mut source = MockNode::<MockNetwork>::with_network_and_blockchain(
        source_network,
        Arc::clone(&source_producer.blockchain),
    );
    let target = Arc::clone(&target_producer.blockchain);
    let target_proxy = BlockchainProxy::from(&target);
    let (mut state_queue, _block_tx) = get_state_queue(Arc::clone(&network), Arc::clone(&target));

    network.dial_mock(&source.network);
    let source_id = source.network.get_local_peer_id();
    state_queue.add_peer(source_id);
    source.request_chunk_handler.pause();

    // Reproduce `MissingNeedsDiff`: the first block has a diff and is adopted, while the two-block
    // suffix arrives diffless with paired chunks and must be returned for retry.
    let initial_item = QueuedStateChunks::Missing(vec![
        (
            (first_block, BlockSource::requested(source_id)),
            Some(first_diff),
            vec![],
        ),
        (
            (first_retry_block, BlockSource::requested(source_id)),
            None,
            vec![ChunkAndSource::new(
                first_retry_chunk.chunk,
                first_retry_chunk.start_key,
                source_id,
            )],
        ),
        (
            (second_retry_block, BlockSource::requested(source_id)),
            None,
            vec![ChunkAndSource::new(
                second_retry_chunk.chunk,
                second_retry_start.clone(),
                source_id,
            )],
        ),
    ]);
    let mut initial_push = StateQueue::push_queue_result(
        Arc::clone(&network),
        target_proxy.clone(),
        Arc::new(Mutex::new(BlsCache::new_test())),
        initial_item,
    );
    assert_eq!(initial_push.len(), 1);
    let initial_result = initial_push.pop_front().unwrap().await;
    assert!(matches!(
        &initial_result,
        PushOpResult::MissingNeedsDiff(
            Ok(ChunksPushResult::EmptyChunks),
            adopted_blocks,
            retry_blocks,
        ) if adopted_blocks == &vec![first_hash.clone()]
            && retry_blocks.len() == 2
            && retry_blocks[0].0.0.hash() == first_retry_hash
            && retry_blocks[0].1.len() == 1
            && retry_blocks[0].1[0].start_key == KeyNibbles::ROOT
            && retry_blocks[1].0.0.hash() == second_retry_hash
            && retry_blocks[1].1.len() == 1
            && retry_blocks[1].1[0].start_key == second_retry_start
    ));
    assert!(target.read().contains(&first_hash, true));
    assert_eq!(target.read().head_hash(), first_hash);
    assert!(!target.read().contains(&first_retry_hash, true));
    assert!(!target.read().contains(&second_retry_hash, true));
    assert!(!target.read().accounts_complete());

    // Handling the result must report only the applied prefix, restore both suffix chunks under
    // their original block hashes, and retry only the suffix.
    assert!(matches!(
        state_queue.process_push_result(initial_result),
        Some(LiveSyncEvent::PushEvent(
            LiveSyncPushEvent::ReceivedMissingBlocks(adopted_blocks)
        )) if adopted_blocks == vec![first_hash.clone()]
    ));
    assert_eq!(state_queue.num_buffered_chunks(), 2);

    let (request, retried_item) = timeout(Duration::from_secs(5), async {
        join!(next(&mut source, 2), state_queue.next())
    })
    .await
    .expect("missing suffix should be retried with a diff");
    assert_eq!(
        request,
        Some(vec![RequestTrieDiff::TYPE_ID, RequestTrieDiff::TYPE_ID])
    );
    let retried_item = retried_item.expect("retry should produce the missing suffix");
    assert!(matches!(
        &retried_item,
        QueuedStateChunks::Missing(blocks)
            if blocks.len() == 2
                && blocks[0].0.0.hash() == first_retry_hash
                && blocks[0].0.1.peer_id() == source_id
                && blocks[0].1.is_some()
                && blocks[0].2.len() == 1
                && blocks[0].2[0].start_key == KeyNibbles::ROOT
                && blocks[0].2[0].peer_id == source_id
                && blocks[1].0.0.hash() == second_retry_hash
                && blocks[1].0.1.peer_id() == source_id
                && blocks[1].1.is_some()
                && blocks[1].2.len() == 1
                && blocks[1].2[0].start_key == second_retry_start
                && blocks[1].2[0].peer_id == source_id
    ));
    assert_eq!(state_queue.num_buffered_chunks(), 0);

    let mut retried_push = StateQueue::push_queue_result(
        network,
        target_proxy,
        Arc::new(Mutex::new(BlsCache::new_test())),
        retried_item,
    );
    assert_eq!(retried_push.len(), 1);
    let final_result = retried_push.pop_front().unwrap().await;
    assert!(matches!(
        &final_result,
        PushOpResult::Missing(
            Ok(PushResult::Extended),
            Ok(ChunksPushResult::Chunks(2, 0)),
            adopted_blocks,
            invalid_blocks,
        ) if adopted_blocks == &vec![first_retry_hash.clone(), second_retry_hash.clone()]
            && invalid_blocks.is_empty()
    ));
    assert!(matches!(
        state_queue.process_push_result(final_result),
        Some(LiveSyncEvent::PushEvent(
            LiveSyncPushEvent::ReceivedMissingBlocks(adopted_blocks)
        )) if adopted_blocks == vec![first_retry_hash.clone(), second_retry_hash.clone()]
    ));

    let target = target.read();
    assert_eq!(target.head_hash(), second_retry_hash);
    assert!(target.contains(&first_hash, true));
    assert!(target.contains(&first_retry_hash, true));
    assert!(target.contains(&second_retry_hash, true));
    assert!(target.accounts_complete());
    assert_eq!(
        target.state.accounts.get_root_hash_assert(None),
        source
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None)
    );
    assert_eq!(state_queue.num_buffered_chunks(), 0);
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
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);

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
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);

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
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);

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
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);

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
            let mut block = blockchain.read().head().clone();
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
            response.as_mut().unwrap().blocks[0]
                .unwrap_micro_ref_mut()
                .body
                .as_mut()
                .unwrap()
                .transactions
                .push(ExecutedTransaction::Ok(
                    TransactionBuilder::new_basic(
                        &KeyPair::default(),
                        Address::burn_address(),
                        Coin::MAX,
                        Coin::ZERO,
                        Policy::genesis_block_number(),
                        NetworkId::UnitAlbatross,
                    )
                    .unwrap(),
                ));
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

        assert_eq!(mock_node.next().await, Some(RequestChunk::TYPE_ID));
        assert_eq!(mock_node.next().await, Some(RequestTrieDiff::TYPE_ID));
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
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);
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
    yield_now().await;

    // Upon first chunk request, return a chunk for a non-existent block.
    mock_node
        .request_chunk_handler
        .set(|mock_id, request, blockchain| {
            let mut chunk = <RequestChunk as Handle<MockNetwork, Arc<RwLock<Blockchain>>>>::handle(
                request, mock_id, blockchain,
            );
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
    yield_now().await;

    // Check buffer.
    assert_eq!(live_sync.queue().num_buffered_chunks(), 1);
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);

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
    yield_now().await;

    // Check buffer.
    assert_eq!(live_sync.queue().num_buffered_chunks(), 1);
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);

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
    yield_now().await;

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
    yield_now().await;

    // Check buffer.
    assert_eq!(live_sync.queue().num_buffered_chunks(), 0);
    assert_eq!(live_sync.queue().num_buffered_heights(), 0);
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

/// Polls the live sync until `condition` holds. Returns `false` once the sync goes quiet for five
/// seconds without the condition being met.
async fn drive_until<F: Fn() -> bool>(
    live_sync: &mut StateLiveSync<MockNetwork>,
    condition: F,
) -> bool {
    for _ in 0..200 {
        if condition() {
            return true;
        }
        match timeout(Duration::from_secs(5), live_sync.next()).await {
            Ok(Some(_)) => {}
            _ => break,
        }
    }
    condition()
}

/// Polls the live sync a few times so that it drains the blockchain events already emitted.
async fn settle(live_sync: &mut StateLiveSync<MockNetwork>) {
    for _ in 0..10 {
        let _ = poll!(live_sync.next());
        yield_now().await;
    }
}

/// An incomplete node and the complete mock peer dialed into it. The peer is neither added to the
/// live sync nor driven yet, so a test can hook its request handlers or drive it by hand first.
struct NodeAndPeer {
    target: Arc<RwLock<Blockchain>>,
    live_sync: StateLiveSync<MockNetwork>,
    network: Arc<MockNetwork>,
    block_tx: Sender<(Block, MockId<MockPeerId>)>,
    mock_node: MockNode<MockNetwork>,
    peer_id: MockPeerId,
    mock_id: MockId<MockPeerId>,
    source: Arc<RwLock<Blockchain>>,
}

async fn incomplete_node_with_complete_peer() -> NodeAndPeer {
    let mut hub = MockHub::new();
    let (target, live_sync, network, block_tx) = get_incomplete_live_sync(&mut hub);

    let info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let mock_node = MockNode::<MockNetwork>::new(
        2,
        info.genesis_block(),
        info.genesis_accounts(),
        &mut Some(hub),
    )
    .await;
    let peer_id = mock_node.network.get_local_peer_id();
    let mock_id = MockId::new(peer_id);
    let source = Arc::clone(&mock_node.blockchain);
    network.dial_mock(&mock_node.network);

    NodeAndPeer {
        target,
        live_sync,
        network,
        block_tx,
        mock_node,
        peer_id,
        mock_id,
        source,
    }
}

struct MidBatchSync {
    target: Arc<RwLock<Blockchain>>,
    live_sync: StateLiveSync<MockNetwork>,
    network: Arc<MockNetwork>,
    peer_id: MockPeerId,
    source: Arc<RwLock<Blockchain>>,
    block_tx: Sender<(Block, MockId<MockPeerId>)>,
    mock_id: MockId<MockPeerId>,
    producer: BlockProducer,
    block1: Block,
    block2: Block,
}

/// Live-syncs an incomplete node from a complete mock peer until its accounts trie completes on
/// micro block 1 and micro block 2 is pushed into the complete trie, both mid-batch.
async fn sync_state_mid_batch() -> MidBatchSync {
    let NodeAndPeer {
        target,
        mut live_sync,
        network,
        block_tx,
        mut mock_node,
        peer_id,
        mock_id,
        source,
    } = incomplete_node_with_complete_peer().await;
    live_sync.add_peer(peer_id);
    spawn(async move { while mock_node.next().await.is_some() {} });

    let producer = BlockProducer::new(signing_key(), voting_key());

    // Block 1 is pushed into the incomplete trie: its chain info records the missing range, so
    // reverting it also reverts the trie chunks committed while it was the head.
    let block1 = push_micro_block(&producer, &source);
    block_tx
        .send((block1.clone(), mock_id.clone()))
        .await
        .unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().accounts_complete()).await,
        "timed out waiting for the state sync"
    );

    // Block 2 is pushed into the complete trie. The state sync must stay pending until the next
    // macro block, since a rebranch can still revert block 1 together with its chunks.
    let block2 = push_micro_block(&producer, &source);
    let height2 = block2.block_number();
    block_tx
        .send((block2.clone(), mock_id.clone()))
        .await
        .unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().block_number() == height2).await,
        "timed out waiting for block 2"
    );
    // Process the `Extended` event of block 2.
    settle(&mut live_sync).await;

    let (head_number, info1, info2) = {
        let target = target.read();
        (
            target.block_number(),
            target.get_chain_info(&block1.hash(), false, None).unwrap(),
            target.get_chain_info(&block2.hash(), false, None).unwrap(),
        )
    };
    assert!(!Policy::is_macro_block_at(head_number));
    assert!(
        !live_sync.queue().chunk_request_state().is_complete(),
        "state sync must not be marked complete before a macro block"
    );
    info!("state sync still pending at micro block #{head_number}");
    assert!(
        info1.prev_missing_range.is_some(),
        "block 1 carries a missing range"
    );
    assert!(
        info2.prev_missing_range.is_none(),
        "block 2 carries no missing range"
    );

    MidBatchSync {
        target,
        live_sync,
        network,
        peer_id,
        source,
        block_tx,
        mock_id,
        producer,
        block1,
        block2,
    }
}

/// A node whose trie completed mid-batch must still adopt a canonical fork that reverts a block of
/// that batch. Reverting block 1 reverts its trie chunks, so the fork blocks can only be committed
/// with their diffs, which the node must keep fetching until the next macro block.
#[test(tokio::test)]
async fn adopts_a_fork_that_reverts_a_block_pushed_into_an_incomplete_trie() {
    let MidBatchSync {
        target,
        mut live_sync,
        source,
        block_tx,
        mock_id,
        block1,
        ..
    } = sync_state_mid_batch().await;

    // A longer competing branch forks right below block 1. The peer adopts it so that it serves
    // those blocks when asked for them.
    let fork_producer = TemporaryBlockProducer::new();
    let fork: Vec<Block> = (0..3)
        .map(|_| fork_producer.next_block(vec![0x42], false))
        .collect();
    for block in &fork {
        Blockchain::push(source.upgradable_read(), block.clone()).unwrap();
    }
    let fork_head = fork.last().unwrap().hash();
    assert_eq!(source.read().head_hash(), fork_head);

    info!(
        "gossiping fork head #{} whose adoption reverts block #{}",
        fork[2].block_number(),
        block1.block_number()
    );
    block_tx.send((fork[2].clone(), mock_id)).await.unwrap();
    let rebranched = drive_until(&mut live_sync, || target.read().head_hash() == fork_head).await;

    let (head, known) = {
        let target = target.read();
        (
            target.head().clone(),
            fork.iter()
                .map(|block| target.contains(&block.hash(), true))
                .collect::<Vec<_>>(),
        )
    };
    info!("rebranched={rebranched} head={head} fork_blocks_known={known:?}");
    assert!(
        rebranched,
        "the node rejected the canonical fork and stayed on the losing branch at {head}"
    );
}

/// Control: the same reorg, but forking above the block that completed the trie. No reverted block
/// carries a missing range, so the rebranch leaves the chunks untouched.
#[test(tokio::test)]
async fn control_adopts_a_fork_that_only_reverts_blocks_pushed_into_a_complete_trie() {
    let MidBatchSync {
        target,
        mut live_sync,
        source,
        block_tx,
        mock_id,
        producer,
        block1,
        block2,
        ..
    } = sync_state_mid_batch().await;

    info!("fork reverts only a block pushed into a complete trie");

    // Block 3 is pushed into the complete trie as well.
    let block3 = push_micro_block(&producer, &source);
    let height3 = block3.block_number();
    block_tx
        .send((block3.clone(), mock_id.clone()))
        .await
        .unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().block_number() == height3).await,
        "timed out waiting for block 3"
    );
    assert!(target
        .read()
        .get_chain_info(&block3.hash(), false, None)
        .unwrap()
        .prev_missing_range
        .is_none());

    // The competing branch forks at block 2, so only block 3 is reverted.
    let fork_producer = TemporaryBlockProducer::new();
    Blockchain::push(fork_producer.blockchain.upgradable_read(), block1).unwrap();
    Blockchain::push(fork_producer.blockchain.upgradable_read(), block2).unwrap();
    let fork: Vec<Block> = (0..2)
        .map(|_| fork_producer.next_block(vec![0x42], false))
        .collect();
    for block in &fork {
        Blockchain::push(source.upgradable_read(), block.clone()).unwrap();
    }
    let fork_head = fork.last().unwrap().hash();
    assert_eq!(source.read().head_hash(), fork_head);

    block_tx.send((fork[1].clone(), mock_id)).await.unwrap();
    let rebranched = drive_until(&mut live_sync, || target.read().head_hash() == fork_head).await;
    let head = target.read().head().clone();
    info!("rebranched={rebranched} head={head}");
    assert!(
        rebranched,
        "control rebranch should succeed, head is {head}"
    );
}

/// After a rebranch applied fork blocks into the incomplete trie and the chunks were synced again,
/// a second rebranch that reverts those fork blocks must revert their chunks before their diffs.
#[test(tokio::test)]
async fn reverts_fork_blocks_that_were_applied_into_an_incomplete_trie() {
    let MidBatchSync {
        target,
        mut live_sync,
        source,
        block_tx,
        mock_id,
        ..
    } = sync_state_mid_batch().await;

    // The first fork reverts block 1, so its blocks are applied into the incomplete trie. Their
    // transactions leave stumps in the missing part that the revert must find again.
    let reward_key = KeyPair::from(PrivateKey::from_str(REWARD_KEY).unwrap());
    let fork_a_producer = TemporaryBlockProducer::new();
    let fork_a: Vec<Block> = (0..3)
        .map(|seed| {
            let txs = generate_transactions(
                &reward_key,
                Policy::genesis_block_number(),
                NetworkId::UnitAlbatross,
                1,
                seed,
            );
            fork_a_producer.next_block_with_txs(vec![0x42], false, txs)
        })
        .collect();
    for block in &fork_a {
        Blockchain::push(source.upgradable_read(), block.clone()).unwrap();
    }
    let fork_a_head = fork_a.last().unwrap().hash();
    block_tx
        .send((fork_a[2].clone(), mock_id.clone()))
        .await
        .unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().head_hash() == fork_a_head).await,
        "timed out waiting for the first rebranch"
    );
    assert!(
        drive_until(&mut live_sync, || target.read().accounts_complete()).await,
        "timed out waiting for the chunks after the first rebranch"
    );

    // A longer branch from the same ancestor reverts the whole first fork.
    let fork_b_producer = TemporaryBlockProducer::new();
    let fork_b: Vec<Block> = (0..4)
        .map(|_| fork_b_producer.next_block(vec![0x43], false))
        .collect();
    for block in &fork_b {
        Blockchain::push(source.upgradable_read(), block.clone()).unwrap();
    }
    let fork_b_head = fork_b.last().unwrap().hash();
    assert_eq!(source.read().head_hash(), fork_b_head);

    info!(
        "gossiping fork head #{} whose adoption reverts fork blocks #{}-#{}",
        fork_b[3].block_number(),
        fork_a[0].block_number(),
        fork_a[2].block_number()
    );
    block_tx.send((fork_b[3].clone(), mock_id)).await.unwrap();
    let rebranched = drive_until(&mut live_sync, || target.read().head_hash() == fork_b_head).await;
    let head = target.read().head().clone();
    info!("rebranched={rebranched} head={head}");
    assert!(
        rebranched,
        "the node failed to revert the fork blocks applied into the incomplete trie, head is {head}"
    );
    assert!(
        target
            .read()
            .get_chain_info(&fork_b[0].hash(), false, None)
            .unwrap()
            .prev_missing_range
            .is_some(),
        "fork block records the missing range it was applied into"
    );
}

/// A fork block stored while the trie is incomplete records the missing range of that moment. If
/// the block is applied later, after a chunk revert cut the trie further back, that stored range
/// is stale: reverting the block would then cut the trie at the stale key and find keys that its
/// revert diff recorded as stumps inside the complete part (`revert_diff` panics with "key should
/// be in complete part"). `rebranch_to` must record the range the block is actually applied into.
#[test(tokio::test)]
async fn fork_block_stored_mid_sync_is_applied_with_the_current_missing_range() {
    let NodeAndPeer {
        target,
        mut live_sync,
        block_tx,
        mut mock_node,
        peer_id,
        mock_id,
        source,
        ..
    } = incomplete_node_with_complete_peer().await;

    let producer = BlockProducer::new(signing_key(), voting_key());

    // Block 1 is pushed into the incomplete trie, then exactly one chunk moves the missing range.
    let block1 = push_micro_block(&producer, &source);
    block_tx
        .send((block1.clone(), mock_id.clone()))
        .await
        .unwrap();
    live_sync.add_peer(peer_id);
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
        "Should receive block 1"
    );
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
        "Should accept the first chunk"
    );
    let frontier = target
        .read()
        .get_missing_accounts_range(None)
        .expect("the trie must still be incomplete after one chunk")
        .start;
    assert_ne!(frontier, KeyNibbles::ROOT);

    // A fork block below block 1 with a transaction to the lowest account key. It is stored into
    // the trie that misses `frontier..`, so its chain info records that range.
    let burn_key = KeyNibbles::from(&Address::burn_address());
    assert!(
        burn_key < frontier,
        "the burn address must sit in the complete part"
    );
    let reward_key = KeyPair::from(PrivateKey::from_str(REWARD_KEY).unwrap());
    let tx = TransactionBuilder::new_basic(
        &reward_key,
        Address::burn_address(),
        Coin::from_u64_unchecked(1),
        Coin::from_u64_unchecked(2),
        Policy::genesis_block_number(),
        NetworkId::UnitAlbatross,
    )
    .unwrap();
    let fork_producer = TemporaryBlockProducer::new();
    let (fork1, fork1_diff) =
        fork_producer.next_block_and_diff_with_txs(vec![0x42], false, vec![tx]);
    let stored = Blockchain::push_with_chunks(
        target.upgradable_read(),
        fork1.clone(),
        fork1_diff,
        vec![],
        &(),
    )
    .map(|(result, _)| result);
    assert!(
        matches!(stored, Ok(PushResult::Ignored | PushResult::Forked)),
        "fork block should be stored as a fork, got {stored:?}"
    );
    Blockchain::push(source.upgradable_read(), fork1.clone()).unwrap();
    let stored_range = target
        .read()
        .get_chain_info(&fork1.hash(), false, None)
        .unwrap()
        .prev_missing_range
        .map(|range| range.start);
    assert_eq!(stored_range, Some(frontier.clone()));
    info!(
        "fork block #{} stored with missing range {frontier}.. while the trie syncs",
        fork1.block_number()
    );

    // Finish the state sync on block 1 and push block 2 into the complete trie.
    spawn(async move { while mock_node.next().await.is_some() {} });
    assert!(
        drive_until(&mut live_sync, || target.read().accounts_complete()).await,
        "timed out waiting for the state sync"
    );
    let block2 = push_micro_block(&producer, &source);
    let height2 = block2.block_number();
    block_tx.send((block2, mock_id.clone())).await.unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().block_number() == height2).await,
        "timed out waiting for block 2"
    );

    // The fork wins. Reverting block 1 reverts its chunks, so the fork is applied into a trie that
    // misses everything: the range stored for fork block 1 is stale now.
    let fork2 = fork_producer.next_block(vec![0x42], false);
    let fork3 = fork_producer.next_block(vec![0x42], false);
    Blockchain::push(source.upgradable_read(), fork2).unwrap();
    assert_eq!(
        Blockchain::push(source.upgradable_read(), fork3.clone()),
        Ok(PushResult::Rebranched)
    );
    block_tx
        .send((fork3.clone(), mock_id.clone()))
        .await
        .unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().head_hash() == fork3.hash()).await,
        "timed out waiting for the first rebranch"
    );
    let applied_range = target
        .read()
        .get_chain_info(&fork1.hash(), false, None)
        .unwrap()
        .prev_missing_range
        .map(|range| range.start);
    info!("fork block applied with missing range {applied_range:?}");
    assert!(
        drive_until(&mut live_sync, || target.read().accounts_complete()).await,
        "timed out re-syncing the chunks after the first rebranch"
    );

    // A longer branch from genesis reverts the fork. Reverting fork block 1 must cut the trie at
    // the range it was applied into, not at the stale stored one.
    let other_producer = TemporaryBlockProducer::new();
    let other: Vec<Block> = (0..4)
        .map(|_| other_producer.next_block(vec![0x43], false))
        .collect();
    for block in &other {
        Blockchain::push(source.upgradable_read(), block.clone()).unwrap();
    }
    let other_head = other.last().unwrap().hash();
    assert_eq!(source.read().head_hash(), other_head);
    info!(
        "gossiping #{} whose adoption reverts the fork block stored with range {frontier}..",
        other[3].block_number()
    );
    block_tx.send((other[3].clone(), mock_id)).await.unwrap();
    let rebranched = drive_until(&mut live_sync, || target.read().head_hash() == other_head).await;
    let head = target.read().head().clone();
    info!("rebranched={rebranched} applied_range={applied_range:?} head={head}");
    assert_eq!(
        applied_range,
        Some(KeyNibbles::ROOT),
        "fork block must record the range it was applied into"
    );
    assert!(
        rebranched,
        "the node failed to revert the fork block, head is {head}"
    );
}

/// Macro sync reinitializes the trie as incomplete without a `Rebranched` event (`push_macro`).
/// The earlier completion must not carry over: once the re-sync completes on a micro block, the
/// gate must stay closed while the blocks of the new batch still carry a missing range.
#[test(tokio::test)]
async fn completion_gate_is_rearmed_after_macro_sync_reinitializes_the_trie() {
    let MidBatchSync {
        target,
        mut live_sync,
        source,
        block_tx,
        mock_id,
        producer,
        ..
    } = sync_state_mid_batch().await;

    // The peer finishes the batch. Its checkpoint is applied here the way macro sync applies it,
    // which reinitializes the accounts trie as incomplete.
    produce_macro_blocks(&producer, &source, 1);
    let macro_block = Block::Macro(source.read().macro_head().clone());
    let macro_number = macro_block.block_number();
    assert!(Policy::is_macro_block_at(macro_number));
    assert_eq!(
        Blockchain::push_macro(target.upgradable_read(), macro_block),
        Ok(PushResult::Extended)
    );
    assert!(!target.read().accounts_complete());
    info!("trie reinitialized as incomplete at macro block #{macro_number}");

    // The first micro block of the new batch is announced before the chunk sync restarts, so the
    // trie completes on a micro block again.
    let block1 = push_micro_block(&producer, &source);
    let height1 = block1.block_number();
    block_tx
        .send((block1.clone(), mock_id.clone()))
        .await
        .unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().block_number() == height1).await,
        "timed out waiting for block #{height1}"
    );
    assert!(
        drive_until(&mut live_sync, || target.read().accounts_complete()).await,
        "timed out re-syncing the chunks"
    );
    assert!(
        target
            .read()
            .get_chain_info(&block1.hash(), false, None)
            .unwrap()
            .prev_missing_range
            .is_some(),
        "block #{height1} must have been pushed into the incomplete trie"
    );

    // The next micro block: the gate must stay closed until the next macro block.
    let block2 = push_micro_block(&producer, &source);
    let height2 = block2.block_number();
    block_tx.send((block2, mock_id)).await.unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().block_number() == height2).await,
        "timed out waiting for block #{height2}"
    );
    settle(&mut live_sync).await;
    let state = live_sync.queue().chunk_request_state().clone();
    info!("state={state:?} head=#{height2}");
    assert!(!Policy::is_macro_block_at(height2));
    assert!(
        !state.is_complete(),
        "state sync was marked complete on micro block #{height2} although the batch holds blocks pushed into the reinitialized trie"
    );
}

/// The completion gate is in-memory, but the missing ranges it protects are persisted with the
/// blocks. A node restarting between the completion of its trie and the next macro block must
/// come back with the gate closed and diffs on: otherwise the first fork reverting a block of that
/// batch is rejected as an invalid fork.
#[test(tokio::test)]
async fn restart_inside_the_completion_window_keeps_the_gate_closed() {
    let MidBatchSync {
        target,
        live_sync,
        network,
        peer_id,
        source,
        block_tx,
        mock_id,
        producer,
        block1,
        ..
    } = sync_state_mid_batch().await;

    // Restart: a fresh state queue over the same chain store, the old one is gone.
    drop(block_tx);
    drop(live_sync);
    let sync_progress = Arc::new(AtomicU32::new(0));
    let (mut live_sync, block_tx) =
        live_sync_over_with_progress(&target, &network, Arc::clone(&sync_progress));
    live_sync.add_peer(peer_id);
    let restarted_at = target.read().block_number();
    assert!(!Policy::is_macro_block_at(restarted_at));
    info!(
        "restarted at micro block #{restarted_at}, block #{} carries a missing range",
        block1.block_number()
    );
    assert!(
        !live_sync.state_complete(),
        "restarted inside the completion window, the state sync must not start complete"
    );
    assert_eq!(
        sync_progress.load(std::sync::atomic::Ordering::Acquire),
        100,
        "the trie is complete, a gate-closed start must not report 0% state sync"
    );

    // Same reorg as `adopts_a_fork_that_reverts_a_block_pushed_into_an_incomplete_trie`.
    let fork_producer = TemporaryBlockProducer::new();
    let fork: Vec<Block> = (0..3)
        .map(|_| fork_producer.next_block(vec![0x42], false))
        .collect();
    for block in &fork {
        Blockchain::push(source.upgradable_read(), block.clone()).unwrap();
    }
    let fork_head = fork.last().unwrap().hash();
    info!(
        "gossiping fork head #{} whose adoption reverts block #{}",
        fork[2].block_number(),
        block1.block_number()
    );
    block_tx
        .send((fork[2].clone(), mock_id.clone()))
        .await
        .unwrap();
    let rebranched = drive_until(&mut live_sync, || target.read().head_hash() == fork_head).await;
    let head = target.read().head().clone();
    info!("rebranched={rebranched} head={head}");
    assert!(
        rebranched,
        "the restarted node rejected the canonical fork and stayed on the losing branch at {head}"
    );

    // The chunks are synced again on the fork; the gate opens at the next macro block only.
    assert!(
        drive_until(&mut live_sync, || target.read().accounts_complete()).await,
        "timed out re-syncing the chunks after the rebranch"
    );
    assert!(!live_sync.state_complete());
    produce_macro_blocks(&producer, &source, 1);
    let macro_block = Block::Macro(source.read().macro_head().clone());
    let macro_number = macro_block.block_number();
    block_tx.send((macro_block, mock_id)).await.unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().block_number()
            == macro_number)
        .await,
        "timed out waiting for macro block #{macro_number}"
    );
    settle(&mut live_sync).await;
    assert!(
        live_sync.state_complete(),
        "state sync must be complete after passing macro block #{macro_number}"
    );
}

/// Control: a restart after the macro block that closed the window starts complete, although the
/// blocks below that macro block still carry their missing ranges.
#[test(tokio::test)]
async fn restart_after_the_macro_block_starts_complete() {
    let MidBatchSync {
        target,
        mut live_sync,
        network,
        source,
        block_tx,
        mock_id,
        producer,
        block1,
        ..
    } = sync_state_mid_batch().await;

    produce_macro_blocks(&producer, &source, 1);
    let macro_block = Block::Macro(source.read().macro_head().clone());
    let macro_number = macro_block.block_number();
    block_tx.send((macro_block, mock_id)).await.unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().block_number()
            == macro_number)
        .await,
        "timed out waiting for macro block #{macro_number}"
    );
    settle(&mut live_sync).await;
    assert!(live_sync.state_complete());
    assert!(target
        .read()
        .get_chain_info(&block1.hash(), false, None)
        .unwrap()
        .prev_missing_range
        .is_some());

    drop(block_tx);
    drop(live_sync);
    let (live_sync, _block_tx) = live_sync_over(&target, &network);
    info!(
        "restarted at macro block #{macro_number}; block #{} below it still carries a missing range",
        block1.block_number()
    );
    assert!(
        live_sync.state_complete(),
        "restarted after the macro block, the state sync must start complete"
    );
}

/// Control: a node whose blocks were all pushed into a complete trie starts complete, which a
/// restarting validator set relies on to establish consensus without waiting for a macro block.
#[test(tokio::test)]
async fn restart_with_a_complete_trie_and_no_incomplete_pushes_starts_complete() {
    let mut hub = MockHub::new();
    let chain = Arc::new(RwLock::new(blockchain(true)));
    let producer = BlockProducer::new(signing_key(), voting_key());
    for _ in 0..2 {
        let block = push_micro_block(&producer, &chain);
        assert!(chain
            .read()
            .get_chain_info(&block.hash(), false, None)
            .unwrap()
            .prev_missing_range
            .is_none());
    }
    let network = Arc::new(hub.new_network());
    let sync_progress = Arc::new(AtomicU32::new(0));
    let (live_sync, _block_tx) =
        live_sync_over_with_progress(&chain, &network, Arc::clone(&sync_progress));
    info!(
        "complete trie, no block of the batch pushed into an incomplete trie, head #{}",
        chain.read().block_number()
    );
    assert!(live_sync.state_complete());
    assert_eq!(
        sync_progress.load(std::sync::atomic::Ordering::Acquire),
        100
    );
}

/// A fork diverging right below a macro block is adopted in one rebranch that ends on that macro
/// block. The push emits `Rebranched` and `Finalized` but no `Extended`, so the gate must open on
/// the `Finalized` event: the macro head is final, waiting for the next one costs a whole batch.
#[test(tokio::test)]
async fn rebranch_onto_a_macro_block_opens_the_gate() {
    let MidBatchSync {
        target,
        mut live_sync,
        source,
        block_tx,
        mock_id,
        producer,
        block1,
        block2,
        ..
    } = sync_state_mid_batch().await;

    // Extend the main chain to the block right below the macro block.
    let mut main_chain = vec![block1, block2];
    while !Policy::is_macro_block_at(source.read().block_number() + 1) {
        main_chain.push(push_micro_block(&producer, &source));
    }
    let last_micro = main_chain.last().unwrap().clone();
    let last_micro_number = last_micro.block_number();
    block_tx.send((last_micro, mock_id.clone())).await.unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().block_number()
            == last_micro_number)
        .await,
        "timed out waiting for micro block #{last_micro_number}"
    );
    settle(&mut live_sync).await;
    assert!(!live_sync.state_complete());

    // The competing branch replaces the last micro block and closes the batch with the macro block.
    let fork_producer = TemporaryBlockProducer::new();
    for block in &main_chain[..main_chain.len() - 1] {
        Blockchain::push(fork_producer.blockchain.upgradable_read(), block.clone()).unwrap();
    }
    let fork_micro = fork_producer.next_block(vec![0x42], false);
    let fork_macro = fork_producer.next_block(vec![0x42], false);
    assert_eq!(fork_micro.block_number(), last_micro_number);
    assert!(fork_macro.is_macro());
    Blockchain::push(source.upgradable_read(), fork_micro).unwrap();
    Blockchain::push(source.upgradable_read(), fork_macro.clone()).unwrap();
    let fork_head = fork_macro.hash();
    assert_eq!(source.read().head_hash(), fork_head);

    info!(
        "gossiping macro block #{} on a fork that replaces micro block #{last_micro_number}",
        fork_macro.block_number()
    );
    block_tx.send((fork_macro, mock_id)).await.unwrap();
    let rebranched = drive_until(&mut live_sync, || target.read().head_hash() == fork_head).await;
    assert!(
        rebranched,
        "the node did not adopt the fork ending on the macro block"
    );
    settle(&mut live_sync).await;
    let complete = live_sync.state_complete();
    info!("rebranched={rebranched} state_complete={complete}");
    assert!(
        complete,
        "the gate must open on the macro block the rebranch ended on, not one batch later"
    );
}

/// Chunk-only pushes emit no blockchain event. A trie that completes while the head is a macro
/// block has nothing left to protect, so the gate must open on the completing chunk instead of
/// waiting for the next block, which on a stalled chain never comes.
#[test(tokio::test)]
async fn trie_completing_on_a_macro_head_opens_the_gate_without_a_new_block() {
    let NodeAndPeer {
        target,
        mut live_sync,
        block_tx: _block_tx,
        mut mock_node,
        peer_id,
        ..
    } = incomplete_node_with_complete_peer().await;
    live_sync.add_peer(peer_id);
    spawn(async move { while mock_node.next().await.is_some() {} });

    // No block is announced: the chunks complete the trie while the genesis macro block is the head.
    assert!(
        drive_until(&mut live_sync, || target.read().accounts_complete()).await,
        "timed out waiting for the state sync"
    );
    let head_number = target.read().block_number();
    assert!(Policy::is_macro_block_at(head_number));
    let complete = live_sync.state_complete();
    info!("state_complete={complete} head=#{head_number}");
    assert!(
        complete,
        "the gate must open on the completing chunk at macro block #{head_number}, no block follows"
    );
}

/// The reported state sync progress (e.g. RPC `getSyncStatus`) must reach 100% once the trie is
/// complete, not stay at the start key of the last chunk received, and start over when a macro
/// sync reinitializes the trie as incomplete.
#[test(tokio::test)]
async fn reports_state_sync_progress() {
    let NodeAndPeer {
        target,
        live_sync,
        network,
        block_tx,
        mut mock_node,
        peer_id,
        mock_id,
        source,
    } = incomplete_node_with_complete_peer().await;
    // Replace the live sync by one that reports its progress.
    drop((live_sync, block_tx));
    let sync_progress = Arc::new(AtomicU32::new(0));
    let (mut live_sync, block_tx) =
        live_sync_over_with_progress(&target, &network, Arc::clone(&sync_progress));
    live_sync.add_peer(peer_id);
    spawn(async move { while mock_node.next().await.is_some() {} });
    let progress = || sync_progress.load(std::sync::atomic::Ordering::Acquire);
    assert_eq!(progress(), 0);

    let producer = BlockProducer::new(signing_key(), voting_key());
    let block = push_micro_block(&producer, &source);
    block_tx.send((block, mock_id.clone())).await.unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().accounts_complete()).await,
        "timed out waiting for the state sync"
    );
    settle(&mut live_sync).await;
    assert_eq!(progress(), 100, "the trie is complete");

    // Macro sync reinitializes the trie as incomplete.
    produce_macro_blocks(&producer, &source, 1);
    let macro_block = Block::Macro(source.read().macro_head().clone());
    assert_eq!(
        Blockchain::push_macro(target.upgradable_read(), macro_block),
        Ok(PushResult::Extended)
    );
    assert!(!target.read().accounts_complete());
    settle(&mut live_sync).await;
    assert!(progress() < 100, "the trie was reinitialized as incomplete");

    // And completes again.
    let block = push_micro_block(&producer, &source);
    block_tx.send((block, mock_id)).await.unwrap();
    assert!(
        drive_until(&mut live_sync, || target.read().accounts_complete()).await,
        "timed out re-syncing the chunks"
    );
    settle(&mut live_sync).await;
    assert_eq!(progress(), 100, "the trie is complete again");
}
