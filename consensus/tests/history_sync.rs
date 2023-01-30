mod sync_utils;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::sync::syncer_proxy::SyncerProxy;
use nimiq_test_log::test;
use nimiq_test_utils::node::TESTING_BLS_CACHE_MAX_CAPACITY;
use nimiq_test_utils::zkp_test_data::{zkp_test_exe, KEYS_PATH};
use nimiq_zkp_component::ZKPComponent;
use parking_lot::{Mutex, RwLock};

use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{Blockchain, BlockchainConfig};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_consensus::consensus::Consensus;
use nimiq_consensus::sync::history::{cluster::SyncCluster, HistoryMacroSync};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::policy::Policy;
use nimiq_test_utils::{
    blockchain::{produce_macro_blocks, signing_key, voting_key},
    test_network::TestNetwork,
};
use nimiq_utils::time::OffsetTime;

use crate::sync_utils::{sync_two_peers, SyncMode};

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
#[ignore]
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
    let env1 = VolatileEnvironment::new(11).unwrap();
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
    let zkp_prover1 = ZKPComponent::new(
        BlockchainProxy::from(&blockchain1),
        Arc::clone(&net1),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        false,
        Some(zkp_test_exe()),
        PathBuf::from(KEYS_PATH),
        None,
    )
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
    let consensus1 = Consensus::from_network(
        blockchain1_proxy.clone(),
        Arc::clone(&net1),
        syncer1,
        zkp_prover1,
    );

    // Setup second peer (not synced yet).
    let env2 = VolatileEnvironment::new(11).unwrap();
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
    let zkp_prover2 = ZKPComponent::new(
        BlockchainProxy::from(&blockchain2),
        Arc::clone(&net2),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
        false,
        Some(zkp_test_exe()),
        PathBuf::from(KEYS_PATH),
        None,
    )
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

    let consensus2 = Consensus::from_network(
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
    tokio::time::sleep(Duration::from_secs(1)).await; // FIXME, Prof. Berrang told me to do this

    // Test ingredients:
    // Request macro chain, first request must return all epochs, but no checkpoint
    let peer_id = net2.get_peers()[0];

    let macro_chain = HistoryMacroSync::request_macro_chain(
        Arc::clone(&net2),
        peer_id,
        vec![consensus2.blockchain.read().head_hash()],
        3,
    )
    .await
    .expect("Should yield macro chain");

    let epochs = macro_chain.epochs.expect("Should contain epochs");
    assert!(
        macro_chain.checkpoint.is_none(),
        "MacroChain must contain either epochs, or a checkpoint or neither"
    );
    let blockchain = consensus1.blockchain.read();
    assert_eq!(epochs.len(), 1);
    assert_eq!(epochs[0], blockchain.election_head_hash());

    // Request epoch 1 using the single epochs election block returned by request_macro_chain
    let epoch = SyncCluster::request_epoch(Arc::clone(&net2), peer_id, epochs[0].clone())
        .await
        .expect("Should yield epoch");
    let block1 = epoch.election_macro_block.expect("Should have block");

    assert_eq!(epoch.total_history_len, 3);
    assert_eq!(
        block1.hash(),
        consensus1.blockchain.read().election_head_hash()
    );

    // Request history chunk of epoch 1.
    let chunk =
        SyncCluster::request_history_chunk(Arc::clone(&net2), peer_id, 1, block1.block_number(), 0)
            .await
            .expect("Should yield history chunk")
            .chunk
            .expect("Should yield history chunk");

    assert_eq!(chunk.history.len(), 3);
    assert_eq!(
        chunk.verify(
            consensus1
                .blockchain
                .read()
                .election_head()
                .header
                .history_root,
            0
        ),
        Some(true)
    );

    // Re-request macro chain. This time it must return no epochs, but the checkpoint.
    let macro_chain = HistoryMacroSync::request_macro_chain(
        Arc::clone(&net2),
        peer_id,
        vec![consensus1.blockchain.read().election_head_hash()],
        3,
    )
    .await
    .expect("Should yield macro chain");
    let checkpoint = macro_chain.checkpoint.expect("Should contain checkpoint");
    assert!(
        macro_chain.epochs.is_none() || macro_chain.epochs.unwrap().len() == 0,
        "MacroChain must contain either epochs, or a checkpoint or neither"
    );
    let blockchain = consensus1.blockchain.read();
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
        .clone()
        .expect("Should have block");

    assert_eq!(epoch.total_history_len, 1);
    assert_eq!(
        block2.hash(),
        consensus1.blockchain.read().macro_head_hash()
    );

    // Request HistoryChunk for epoch 2 containing the checkpoint
    let chunk =
        SyncCluster::request_history_chunk(Arc::clone(&net2), peer_id, 2, block2.block_number(), 0)
            .await
            .expect("Should yield history chunk")
            .chunk
            .expect("Should yield history chunk");

    assert_eq!(chunk.history.len(), 1);
    assert_eq!(
        chunk.verify(
            consensus1
                .blockchain
                .read()
                .macro_head()
                .header
                .history_root,
            0
        ),
        Some(true)
    );

    // Re-request Macro chain one final time. This time it must return neither epochs, nor a checkpoint.
    let macro_chain = HistoryMacroSync::request_macro_chain(
        Arc::clone(&net2),
        peer_id,
        vec![consensus1.blockchain.read().macro_head_hash()],
        3,
    )
    .await
    .expect("Should yield macro chain");
    assert!(
        macro_chain.epochs.is_none() || macro_chain.epochs.unwrap().len() == 0,
        "Must not contain epochs"
    );
    assert!(
        macro_chain.checkpoint.is_none(),
        "Must not contain a checkpoint"
    );
}
