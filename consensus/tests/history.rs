use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::{Stream, StreamExt};
use parking_lot::RwLock;

use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_consensus::consensus::Consensus;
use nimiq_consensus::sync::history::{cluster::SyncCluster, HistorySync, HistorySyncReturn};
use nimiq_consensus::sync::request_component::HistorySyncStream;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::Network;
use nimiq_network_mock::MockHub;
use nimiq_primitives::policy;
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{produce_macro_blocks, signing_key, voting_key},
    test_network::TestNetwork,
};
use nimiq_utils::time::OffsetTime;

pub struct MockHistorySyncStream<TNetwork: NetworkInterface> {
    _network: Arc<TNetwork>,
}

impl<TNetwork: NetworkInterface> HistorySyncStream<TNetwork::PeerId>
    for MockHistorySyncStream<TNetwork>
{
    fn add_peer(&self, _peer_id: TNetwork::PeerId) {}
}

impl<TNetwork: NetworkInterface> Stream for MockHistorySyncStream<TNetwork> {
    type Item = HistorySyncReturn<TNetwork::PeerId>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

#[test(tokio::test)]
async fn peers_can_sync() {
    let hub = MockHub::default();
    let mut networks = vec![];

    // Setup first peer.
    let env1 = VolatileEnvironment::new(10).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(env1.clone(), NetworkId::UnitAlbatross, time).unwrap(),
    ));

    let producer = BlockProducer::new(signing_key(), voting_key());

    // The minimum number of macro blocks necessary so that we have one election block and one
    // checkpoint block to push.
    let num_macro_blocks = (policy::BATCHES_PER_EPOCH + 1) as usize;

    // Produce the blocks.
    produce_macro_blocks(&producer, &blockchain1, num_macro_blocks);

    let net1 = TestNetwork::build_network(0, Default::default(), &mut Some(hub)).await;
    networks.push(Arc::clone(&net1));
    let sync1 = HistorySync::<Network>::new(
        Arc::clone(&blockchain1),
        Arc::clone(&net1),
        net1.subscribe_events(),
    );
    let consensus1 =
        Consensus::from_network(env1, blockchain1, Arc::clone(&net1), Box::pin(sync1)).await;

    // Setup second peer (not synced yet).
    let time = Arc::new(OffsetTime::new());
    let env2 = VolatileEnvironment::new(10).unwrap();
    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(env2.clone(), NetworkId::UnitAlbatross, time).unwrap(),
    ));

    let net2 =
        TestNetwork::build_network(1, Default::default(), &mut Some(MockHub::default())).await;
    networks.push(Arc::clone(&net2));
    let mut sync2 = HistorySync::<Network>::new(
        Arc::clone(&blockchain2),
        Arc::clone(&net2),
        net2.subscribe_events(),
    );
    let consensus2 = Consensus::from_network(
        env2,
        blockchain2,
        Arc::clone(&net2),
        Box::pin(MockHistorySyncStream {
            _network: Arc::clone(&net2),
        }),
    )
    .await;

    Network::connect_networks(&networks, 1u64).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let sync_result = sync2.next().await;

    assert!(sync_result.is_some());
    assert_eq!(
        consensus2.blockchain.read().election_head_hash(),
        consensus1.blockchain.read().election_head_hash(),
    );
    assert_eq!(
        consensus2.blockchain.read().macro_head_hash(),
        consensus1.blockchain.read().macro_head_hash(),
    );

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
    let env1 = VolatileEnvironment::new(10).unwrap();
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(env1.clone(), NetworkId::UnitAlbatross, time).unwrap(),
    ));

    let producer = BlockProducer::new(signing_key(), voting_key());

    // The minimum number of macro blocks necessary so that we have one election block and one
    // checkpoint block to push.
    let num_macro_blocks = (policy::BATCHES_PER_EPOCH + 1) as usize;

    // Produce the blocks.
    produce_macro_blocks(&producer, &blockchain1, num_macro_blocks);

    let net1 = TestNetwork::build_network(2, Default::default(), &mut Some(hub)).await;
    networks.push(Arc::clone(&net1));
    let consensus1 = Consensus::from_network(
        env1,
        blockchain1,
        Arc::clone(&net1),
        Box::pin(MockHistorySyncStream {
            _network: Arc::clone(&net1),
        }),
    )
    .await;

    // Setup second peer (not synced yet).
    let env2 = VolatileEnvironment::new(10).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(env2.clone(), NetworkId::UnitAlbatross, time).unwrap(),
    ));

    let net2: Arc<Network> =
        TestNetwork::build_network(3, Default::default(), &mut Some(MockHub::default())).await;
    networks.push(Arc::clone(&net2));
    let consensus2 = Consensus::from_network(
        env2,
        blockchain2,
        Arc::clone(&net2),
        Box::pin(MockHistorySyncStream {
            _network: Arc::clone(&net2),
        }),
    )
    .await;

    // Connect the two peers.
    let mut stream = net2.subscribe_events();
    Network::connect_networks(&networks, 3u64).await;
    // Then wait for connection to be established.
    let _ = stream.next().await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await; // FIXME, Prof. Berrang told me to do this

    // Test ingredients:
    // Request macro chain
    let peer_id = net2.get_peers()[0];
    let macro_chain = HistorySync::request_macro_chain(
        Arc::clone(&net2),
        peer_id,
        vec![consensus2.blockchain.read().head_hash()],
        3,
    )
    .await
    .expect("Should yield macro chain");
    let epochs = macro_chain.epochs.expect("Should contain epochs");
    let checkpoint = macro_chain.checkpoint.expect("Should contain checkpoint");
    let blockchain = consensus1.blockchain.read();
    assert_eq!(epochs.len(), 1);
    assert_eq!(epochs[0], blockchain.election_head_hash());
    assert_eq!(checkpoint.hash, blockchain.macro_head_hash());

    // Request epoch
    let epoch = SyncCluster::request_epoch(
        Arc::clone(&net2),
        peer_id,
        consensus1.blockchain.read().election_head_hash(),
    )
    .await
    .expect("Should yield epoch");
    let block1 = epoch.block.expect("Should have block");

    assert_eq!(epoch.history_len, 3);
    assert_eq!(
        block1.hash(),
        consensus1.blockchain.read().election_head_hash()
    );

    let epoch = SyncCluster::request_epoch(
        Arc::clone(&net2),
        peer_id,
        consensus1.blockchain.read().macro_head_hash(),
    )
    .await
    .expect("Should yield epoch");
    let block2 = epoch.block.expect("Should have block");

    assert_eq!(epoch.history_len, 1);
    assert_eq!(
        block2.hash(),
        consensus1.blockchain.read().macro_head_hash()
    );

    // Request history chunk.
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
}
