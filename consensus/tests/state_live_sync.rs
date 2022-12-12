use futures::{poll, StreamExt};
use log::info;
use nimiq_test_utils::block_production::TemporaryBlockProducer;
use parking_lot::{Mutex, RwLock};
use std::sync::Arc;
use std::task::Poll;
use tokio::sync::mpsc::{self, Sender};
use tokio_stream::wrappers::ReceiverStream;

use nimiq_block::Block;
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::{AbstractBlockchain, PushResult};
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::sync::syncer::{LiveSyncEvent, LiveSyncPushEvent};
use nimiq_consensus::sync::{
    live::{
        block_queue::{block_request_component::BlockRequestComponent, BlockQueue},
        queue::QueueConfig,
        state_queue::StateQueue,
        StateLiveSync,
    },
    syncer::LiveSync,
};
use nimiq_database::{volatile::VolatileEnvironment, WriteTransaction};
use nimiq_genesis::{NetworkId, NetworkInfo};
use nimiq_network_interface::network::Network;
use nimiq_network_mock::{MockHub, MockId, MockNetwork, MockPeerId};
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::blockchain::produce_macro_blocks;
use nimiq_test_utils::{
    blockchain::{push_micro_block, signing_key, voting_key},
    node::{Node, TESTING_BLS_CACHE_MAX_CAPACITY},
};
use nimiq_utils::math::CeilingDiv;
use nimiq_utils::time::OffsetTime;

fn blockchain(complete: bool) -> Arc<RwLock<Blockchain>> {
    let time = Arc::new(OffsetTime::new());
    let env = VolatileEnvironment::new(10).unwrap();
    let blockchain = Blockchain::new(
        env.clone(),
        BlockchainConfig::default(),
        NetworkId::UnitAlbatross,
        time,
    )
    .unwrap();

    if !complete {
        let mut txn = WriteTransaction::new(&env);
        blockchain
            .state
            .accounts
            .reinitialize_as_incomplete(&mut txn);
        txn.commit();
    }

    Arc::new(RwLock::new(blockchain))
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
    StateLiveSync<MockNetwork, BlockRequestComponent<MockNetwork>>,
    Arc<MockNetwork>,
    Sender<(Block, MockId<MockPeerId>)>,
) {
    let incomplete_blockchain = blockchain(false);
    let incomplete_blockchain_proxy = BlockchainProxy::from(&incomplete_blockchain);

    let network = Arc::new(hub.new_network_with_address(1));
    let request_component =
        BlockRequestComponent::new(network.subscribe_events(), Arc::clone(&network), true);
    let (block_tx, block_rx) = mpsc::channel(32);

    let block_queue = BlockQueue::with_gossipsub_block_stream(
        incomplete_blockchain_proxy.clone(),
        Arc::clone(&network),
        request_component,
        ReceiverStream::new(block_rx).boxed(),
        QueueConfig::default(),
    );

    let state_queue = StateQueue::with_block_queue(
        Arc::clone(&incomplete_blockchain),
        Arc::clone(&network),
        block_queue,
        QueueConfig::default(),
    );

    let live_sync = StateLiveSync::with_queue(
        incomplete_blockchain_proxy.clone(),
        Arc::clone(&network),
        state_queue,
        bls_cache(),
    );

    (incomplete_blockchain, live_sync, network, block_tx)
}

async fn gossip_head_block(
    block_tx: &Sender<(Block, MockId<MockPeerId>)>,
    node: &Node<MockNetwork>,
) {
    let mock_id = MockId::new(node.network.get_local_peer_id());
    let block = node.blockchain.read().state.main_chain.head.clone();
    block_tx.send((block, mock_id)).await.unwrap();
}

#[test(tokio::test)]
async fn can_sync_state() {
    let mut hub = MockHub::new();

    // Setup the incomplete node.
    let (incomplete_blockchain, mut live_sync, network, block_tx) =
        get_incomplete_live_sync(&mut hub);

    // Setup the complete node.
    let network_info = NetworkInfo::from_network_id(NetworkId::UnitAlbatross);
    let genesis_block = network_info.genesis_block::<Block>();
    let genesis_accounts = network_info.genesis_accounts();
    let mut complete_node =
        Node::<MockNetwork>::new_history(2, genesis_block, genesis_accounts, &mut Some(hub), false)
            .await;

    // Connect the nodes.
    network.dial_mock(&complete_node.network);

    // Produce a couple of blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());
    push_micro_block(&producer, &complete_node.blockchain);
    gossip_head_block(&block_tx, &complete_node).await;

    // Start complete node.
    complete_node.consume();

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
    live_sync.add_peer(complete_node.network.get_local_peer_id());

    // Will request chunks and receive the block.
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(
                LiveSyncPushEvent::AcceptedAnnouncedBlock(_)
            ))
        ),
        "Should immediately receive block"
    );
    assert_eq!(live_sync.queue().buffered_chunks_len(), 0);
    assert_eq!(live_sync.queue().buffered_blocks_len(), 0);

    let size = complete_node.blockchain.read().state.accounts.size();
    let num_chunks = size.ceiling_div(Policy::state_chunks_max_size() as u64);
    for i in 0..num_chunks {
        info!("Applying chunk #{}", i);
        assert!(
            matches!(
                live_sync.next().await,
                Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                    _
                )))
            ),
            "Should receive and accept chunks"
        );
        assert_eq!(live_sync.queue().buffered_chunks_len(), 0);
        assert_eq!(live_sync.queue().buffered_blocks_len(), 0);

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
    assert!(blockchain_rg.state.accounts.is_complete(None));
    assert!(!live_sync.queue().chunk_request_state().is_complete());
    drop(blockchain_rg);

    produce_macro_blocks(&producer, &complete_node.blockchain, 1);
    gossip_head_block(&block_tx, &complete_node).await;

    // Will request missing blocks and apply those.
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(
                LiveSyncPushEvent::ReceivedMissingBlocks(..)
            ))
        ),
        "Should receive missing blocks"
    );
    assert_eq!(live_sync.queue().buffered_chunks_len(), 0);
    assert_eq!(live_sync.queue().buffered_blocks_len(), 1);

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
    assert_eq!(live_sync.queue().buffered_chunks_len(), 0);
    assert_eq!(live_sync.queue().buffered_blocks_len(), 0);

    push_micro_block(&producer, &complete_node.blockchain);
    gossip_head_block(&block_tx, &complete_node).await;

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
    assert_eq!(live_sync.queue().buffered_chunks_len(), 0);
    assert_eq!(live_sync.queue().buffered_blocks_len(), 0);

    let blockchain_rg = incomplete_blockchain.read();
    assert!(blockchain_rg.state.accounts.is_complete(None));
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
    let genesis_block = network_info.genesis_block::<Block>();
    let genesis_accounts = network_info.genesis_accounts();
    let mut complete_node =
        Node::<MockNetwork>::new_history(2, genesis_block, genesis_accounts, &mut Some(hub), false)
            .await;

    // Connect the nodes.
    network.dial_mock(&complete_node.network);

    // Produce a couple of blocks.
    let producer = BlockProducer::new(signing_key(), voting_key());
    let producer2 = TemporaryBlockProducer::new();

    push_micro_block(&producer, &complete_node.blockchain);
    gossip_head_block(&block_tx, &complete_node).await;

    let fork1a = producer2.next_block(vec![0x48], false);
    let fork1b = producer2.next_block(vec![], false);

    // Start complete node.
    complete_node.consume();

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
    live_sync.add_peer(complete_node.network.get_local_peer_id());

    // Will request a chunk and receive the block.
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(
                LiveSyncPushEvent::AcceptedAnnouncedBlock(_)
            ))
        ),
        "Should immediately receive block"
    );
    assert_eq!(live_sync.queue().buffered_chunks_len(), 0);
    assert_eq!(live_sync.queue().buffered_blocks_len(), 0);

    info!("Applying chunk #{}", 0);
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                _
            )))
        ),
        "Should receive and accept chunks"
    );
    assert_eq!(live_sync.queue().buffered_chunks_len(), 0);
    assert_eq!(live_sync.queue().buffered_blocks_len(), 0);

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
        Blockchain::push(complete_node.blockchain.upgradable_read(), fork1a),
        Ok(PushResult::Forked),
    );
    assert_eq!(
        Blockchain::push(complete_node.blockchain.upgradable_read(), fork1b),
        Ok(PushResult::Rebranched),
    );

    info!("Requesting chunks #{}", 1);
    assert!(matches!(poll!(live_sync.next()), Poll::Pending));
    assert!(matches!(poll!(live_sync.next()), Poll::Pending));

    gossip_head_block(&block_tx, &complete_node).await;

    // Will request a chunk and receive the block.
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(
                LiveSyncPushEvent::ReceivedMissingBlocks(..)
            ))
        ),
        "Should immediately receive block"
    );
    assert_eq!(live_sync.queue().buffered_chunks_len(), 1);
    assert_eq!(live_sync.queue().buffered_blocks_len(), 1);

    info!("Applying chunk #{}", 1);
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(
                LiveSyncPushEvent::AcceptedBufferedBlock(..)
            ))
        ),
        "Should receive and accept chunks"
    );
    assert_eq!(live_sync.queue().buffered_chunks_len(), 0);
    assert_eq!(live_sync.queue().buffered_blocks_len(), 0);

    info!("Applying chunk #{}", 0);
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                ..
            )))
        ),
        "Should receive and accept chunks"
    );

    info!("Applying chunk #{}", 1);
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                ..
            )))
        ),
        "Should receive and accept chunks"
    );

    info!("Applying chunk #{}", 2);
    assert!(
        matches!(
            live_sync.next().await,
            Some(LiveSyncEvent::PushEvent(LiveSyncPushEvent::AcceptedChunks(
                ..
            )))
        ),
        "Should receive and accept chunks"
    );

    // Checks that the state is complete but that the sync is still incomplete
    // and waiting for the macro block.
    let blockchain_rg = incomplete_blockchain.read();
    assert!(blockchain_rg.state.accounts.is_complete(None));
    assert!(!live_sync.queue().chunk_request_state().is_complete());
    assert_eq!(
        blockchain_rg.state.accounts.get_root_hash_assert(None),
        complete_node
            .blockchain
            .read()
            .state
            .accounts
            .get_root_hash_assert(None),
        "Final accounts tries should be the same"
    );
    drop(blockchain_rg);
}

// TODO Tests:
//
// Add chunks from different blocks with account trie changes on known and unknown parts of the trie
// Add invalid chunks
// Remove chunks related to invalid blocks
// Test the reset chain of chunks
// Buffer clearing after macro blocks
