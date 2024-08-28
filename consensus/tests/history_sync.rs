use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use nimiq_blockchain::{BlockProducer, Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::{
    consensus::Consensus,
    sync::{
        history::{
            cluster::{HistoryChunkRequest, SyncCluster},
            HistoryMacroSync,
        },
        syncer_proxy::SyncerProxy,
    },
};
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis::NetworkId;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::policy::Policy;
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{produce_macro_blocks, signing_key, voting_key},
    node::TESTING_BLS_CACHE_MAX_CAPACITY,
    test_network::TestNetwork,
};
use nimiq_time::sleep;
use nimiq_utils::time::OffsetTime;
use nimiq_zkp_component::ZKPComponent;
use parking_lot::{Mutex, RwLock};

use crate::sync_utils::{sync_two_peers, SyncMode};

mod sync_utils;

#[test(tokio::test)]
async fn two_peers_can_sync_empty_chain() {
    sync_two_peers(0, 0, SyncMode::History).await
}

#[test(tokio::test)]
async fn two_peers_can_sync_single_batch() {
    sync_two_peers(1, 0, SyncMode::History).await
}

#[test(tokio::test)]
async fn two_peers_can_sync_two_batches() {
    sync_two_peers(2, 0, SyncMode::History).await
}

#[test(tokio::test)]
async fn two_peers_can_sync_epoch_minus_batch() {
    sync_two_peers(
        (Policy::batches_per_epoch() - 1) as usize,
        0,
        SyncMode::History,
    )
    .await
}

#[test(tokio::test)]
async fn two_peers_can_sync_epoch_plus_batch() {
    sync_two_peers(
        (Policy::batches_per_epoch() + 1) as usize,
        0,
        SyncMode::History,
    )
    .await
}

#[test(tokio::test)]
async fn two_peers_can_sync_epoch_plus_two_batches() {
    sync_two_peers(
        (Policy::batches_per_epoch() + 2) as usize,
        0,
        SyncMode::History,
    )
    .await
}

#[test(tokio::test)]
async fn two_peers_can_sync_single_epoch() {
    sync_two_peers(Policy::batches_per_epoch() as usize, 0, SyncMode::History).await
}

#[test(tokio::test)]
async fn two_peers_can_sync_two_epochs() {
    sync_two_peers(
        (Policy::batches_per_epoch() * 2) as usize,
        0,
        SyncMode::History,
    )
    .await
}

#[test(tokio::test)]
async fn three_peers_can_sync() {
    // FIXME: Add more tests
    //    // Setup third peer (not synced yet).
    //    let env3 = VolatileEnvironment::new(10).unwrap();
    //    let blockchain3 = Arc::new(Blockchain::new(env3.clone(), NetworkId::UnitAlbatross).unwrap());
    //    let mempool3 = Mempool::new(Arc::clone(&blockchain3), MempoolConfig::default());
    //
    //    let net3 = Arc::new(MockNetwork::new(3));
    //    let sync3 = QuickSync::default();
    //    let consensus3 = Consensus::new(env3, blockchain3, mempool3, Arc::clone(&net3), sync3).unwrap();
    //
    //    // Third peer has two micro blocks that need to be reverted.
    //    for i in 1..4 {
    //        consensus3
    //            .blockchain
    //            .push(
    //                consensus1
    //                    .blockchain
    //                    .chain_store
    //                    .get_block_at(i, true, None)
    //                    .unwrap(),
    //            )
    //            .unwrap();
    //    }
    //
    //    // Connect the new peer with macro synced peer.
    //    net3.connect(&net2);
    //    // Then wait for connection to be established.
    //    let mut stream = consensus3.subscribe_events();
    //    stream.recv().await;
    //
    //    assert_eq!(consensus3.num_agents(), 1);
    //
    //    // Test ingredients:
    //    // Request hashes
    //    let agent = Arc::clone(consensus3.agents().values().next().unwrap());
    //    let hashes = agent
    //        .request_block_hashes(
    //            vec![consensus3.blockchain.head_hash()],
    //            2,
    //            RequestBlockHashesFilter::ElectionOnly,
    //        )
    //        .await
    //        .expect("Should yield hashes");
    //    assert_eq!(hashes.hashes.len(), 1);
    //    assert_eq!(
    //        hashes.hashes[0].1,
    //        consensus2.blockchain.election_head_hash()
    //    );
    //
    //    // Request epoch
    //    let epoch = agent
    //        .request_epoch(consensus2.blockchain.election_head_hash())
    //        .await
    //        .expect("Should yield epoch");
    //    assert_eq!(epoch.history_len, 0);
    //    assert_eq!(
    //        epoch.block.hash(),
    //        consensus2.blockchain.election_head_hash()
    //    );
    //
    //    let sync_result = Consensus::sync_blockchain(Arc::downgrade(&consensus3)).await;
    //
    //    assert!(sync_result.is_ok());
    //    assert_eq!(
    //        consensus3.blockchain.election_head_hash(),
    //        consensus1.blockchain.election_head_hash()
    //    );
}

#[test(tokio::test)]
async fn sync_ingredients() {
    let hub = MockHub::default();
    let mut networks = vec![];

    // Setup first peer.
    let time = Arc::new(OffsetTime::new());
    let env1 = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(
            env1.clone(),
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));

    let producer = BlockProducer::new(signing_key(), voting_key());

    // The minimum number of macro blocks necessary so that we have one election block and one
    // checkpoint block to push.
    let num_macro_blocks = (Policy::batches_per_epoch() + 1) as usize;

    // Produce the blocks.
    produce_macro_blocks(&producer, &blockchain1, num_macro_blocks);

    let net1: Arc<Network> =
        TestNetwork::build_network(2, Default::default(), &mut Some(hub)).await;
    networks.push(Arc::clone(&net1));
    let zkp_prover1 =
        ZKPComponent::new(BlockchainProxy::from(&blockchain1), Arc::clone(&net1), None)
            .await
            .proxy();
    let blockchain1_proxy = BlockchainProxy::from(&blockchain1);
    let syncer1 = SyncerProxy::new_history(
        blockchain1_proxy.clone(),
        Arc::clone(&net1),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
        net1.subscribe_events(),
    )
    .await;
    // The consensus itself is unused, but in from_network the request handlers are
    // spawned and thus this needs to execute as they are used later.
    let _consensus1 = Consensus::from_network(
        blockchain1_proxy.clone(),
        Arc::clone(&net1),
        syncer1,
        zkp_prover1,
    );

    // Setup second peer (not synced yet).
    let env2 = MdbxDatabase::new_volatile(Default::default()).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(
            env2.clone(),
            BlockchainConfig::default(),
            NetworkId::UnitAlbatross,
            time,
        )
        .unwrap(),
    ));

    let net2: Arc<Network> =
        TestNetwork::build_network(3, Default::default(), &mut Some(MockHub::default())).await;
    networks.push(Arc::clone(&net2));
    let zkp_prover2 =
        ZKPComponent::new(BlockchainProxy::from(&blockchain2), Arc::clone(&net2), None)
            .await
            .proxy();
    let blockchain2_proxy = BlockchainProxy::from(&blockchain2);
    let syncer2 = SyncerProxy::new_history(
        blockchain2_proxy.clone(),
        Arc::clone(&net2),
        Arc::new(Mutex::new(PublicKeyCache::new(
            TESTING_BLS_CACHE_MAX_CAPACITY,
        ))),
        net2.subscribe_events(),
    )
    .await;
    // The consensus itself is unused, but in from_network the request handlers are
    // spawned and thus this needs to execute as they are used later.
    let _consensus2 = Consensus::from_network(
        blockchain2_proxy.clone(),
        Arc::clone(&net2),
        syncer2,
        zkp_prover2,
    );

    // Connect the two peers.
    let mut stream = net2.subscribe_events();
    Network::connect_networks(&networks, 3u64).await;
    // Then wait for connection to be established.
    let _ = stream.next().await.unwrap();
    sleep(Duration::from_secs(1)).await; // FIXME, Prof. Berrang told me to do this

    // Test ingredients:
    // Request macro chain, first request must return all epochs and one checkpoint.
    let peer_id = net2.get_peers()[0];

    let head_hash = blockchain2.read().head_hash();

    let macro_chain =
        HistoryMacroSync::request_macro_chain(Arc::clone(&net2), peer_id, vec![head_hash], 3)
            .await
            .expect("Should yield macro chain")
            .expect("Should yield macro chain 2");

    assert!(
        macro_chain.checkpoint.is_some(),
        "Should contain checkpoint"
    );
    let blockchain = blockchain1.read();
    assert_eq!(macro_chain.epochs.len(), 1);
    assert_eq!(macro_chain.epochs[0], blockchain.election_head_hash());

    // Request epoch 1 using the single epochs election block returned by request_macro_chain
    let epoch =
        SyncCluster::request_epoch(Arc::clone(&net2), peer_id, macro_chain.epochs[0].clone())
            .await
            .expect("Should yield epoch");
    let block1 = epoch
        .election_macro_block
        .as_ref()
        .expect("Should have block");

    assert_eq!(epoch.total_history_len(), 3);
    assert_eq!(block1.hash(), blockchain1.read().election_head_hash());

    // Request history chunk of epoch 1.
    let chunk = SyncCluster::request_history_chunk(
        Arc::clone(&net2),
        peer_id,
        HistoryChunkRequest::from_block(block1, 0),
    )
    .await
    .expect("Should yield history chunk");

    assert_eq!(chunk.history.len(), 3);
    assert_eq!(
        chunk.verify(&blockchain1.read().election_head().header.history_root, 0),
        Some(true)
    );

    // Re-request macro chain. This time it must return no epochs, but the checkpoint.
    let macro_chain = HistoryMacroSync::request_macro_chain(
        Arc::clone(&net2),
        peer_id,
        vec![blockchain1.read().election_head_hash()],
        3,
    )
    .await
    .expect("Should yield macro chain")
    .expect("Should yield macro chain 2");
    let checkpoint = macro_chain.checkpoint.expect("Should contain checkpoint");
    assert!(macro_chain.epochs.is_empty(), "Should not contain epochs");
    let blockchain = blockchain1.read();
    assert_eq!(checkpoint.hash, blockchain.macro_head_hash());

    // Request epoch 2 using the returned checkpoint
    let epoch = SyncCluster::request_epoch(Arc::clone(&net2), peer_id, checkpoint.hash)
        .await
        .expect("Should yield epoch");
    let block2 = epoch
        .batch_sets
        .last()
        .expect("Should have a batch set")
        .macro_block
        .clone();

    assert_eq!(epoch.total_history_len(), 1);
    assert_eq!(block2.hash(), blockchain1.read().macro_head_hash());

    // Request HistoryChunk for epoch 2 containing the checkpoint
    let chunk = SyncCluster::request_history_chunk(
        Arc::clone(&net2),
        peer_id,
        HistoryChunkRequest::from_block(&block2, 0),
    )
    .await
    .expect("Should yield history chunk");

    assert_eq!(chunk.history.len(), 1);
    assert_eq!(
        chunk.verify(&blockchain1.read().macro_head().header.history_root, 0),
        Some(true)
    );

    // Re-request Macro chain one final time. This time it must return neither epochs, nor a checkpoint.
    let macro_chain = HistoryMacroSync::request_macro_chain(
        Arc::clone(&net2),
        peer_id,
        vec![blockchain1.read().macro_head_hash()],
        3,
    )
    .await
    .expect("Should yield macro chain")
    .expect("Should yield macro chain 2");
    assert!(macro_chain.epochs.is_empty(), "Must not contain epochs");
    assert!(
        macro_chain.checkpoint.is_none(),
        "Must not contain a checkpoint"
    );
}
