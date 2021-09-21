use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures::task::{Context, Poll};
use futures::{Stream, StreamExt};
use parking_lot::RwLock;

use beserial::Deserialize;
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_consensus::consensus::Consensus;
use nimiq_consensus::consensus_agent::ConsensusAgent;
use nimiq_consensus::messages::RequestBlockHashesFilter;
use nimiq_consensus::sync::history::HistorySync;
use nimiq_consensus::sync::request_component::HistorySyncStream;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis::NetworkId;
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_network_interface::network::Network;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::policy;
use nimiq_test_utils::blockchain::produce_macro_blocks;
use nimiq_utils::time::OffsetTime;

pub struct MockHistorySyncStream<TNetwork: Network> {
    network: Arc<TNetwork>,
}

impl<TNetwork: Network> HistorySyncStream<TNetwork::PeerType> for MockHistorySyncStream<TNetwork> {
    fn add_peer(&self, _peer: Arc<TNetwork::PeerType>) {}
}

impl<TNetwork: Network> Stream for MockHistorySyncStream<TNetwork> {
    type Item = Arc<ConsensusAgent<TNetwork::PeerType>>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

/// Secret key of validator. Tests run with `network-primitives/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str =
    "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

#[tokio::test]
async fn peers_can_sync() {
    //simple_logger::SimpleLogger::new().init().unwrap();

    let mut hub = MockHub::default();

    // Setup first peer.
    let env1 = VolatileEnvironment::new(10).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(env1.clone(), NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let mempool1 = Mempool::new(Arc::clone(&blockchain1), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(
        Arc::clone(&blockchain1),
        Arc::clone(&mempool1),
        keypair.clone(),
    );

    // The minimum number of macro blocks necessary so that we have one election block and one
    // checkpoint block to push.
    let num_macro_blocks = (policy::BATCHES_PER_EPOCH + 1) as usize;

    // Produce the blocks.
    produce_macro_blocks(num_macro_blocks, &producer, &blockchain1);

    let net1 = Arc::new(hub.new_network());
    let sync1 = HistorySync::<MockNetwork>::new(Arc::clone(&blockchain1), net1.subscribe_events());
    let consensus1 = Consensus::from_network(
        env1,
        blockchain1,
        mempool1,
        Arc::clone(&net1),
        Box::pin(sync1),
    )
    .await;

    // Setup second peer (not synced yet).
    let time = Arc::new(OffsetTime::new());
    let env2 = VolatileEnvironment::new(10).unwrap();
    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(env2.clone(), NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let mempool2 = Mempool::new(Arc::clone(&blockchain2), MempoolConfig::default());

    let net2 = Arc::new(hub.new_network());
    let mut sync2 =
        HistorySync::<MockNetwork>::new(Arc::clone(&blockchain2), net2.subscribe_events());
    let consensus2 = Consensus::from_network(
        env2,
        blockchain2,
        mempool2,
        Arc::clone(&net2),
        Box::pin(MockHistorySyncStream {
            network: Arc::clone(&net2),
        }),
    )
    .await;

    net1.dial_mock(&net2);
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

#[tokio::test]
async fn sync_ingredients() {
    //simple_logger::SimpleLogger::new().init().unwrap();
    let mut hub = MockHub::default();

    // Setup first peer.
    let time = Arc::new(OffsetTime::new());
    let env1 = VolatileEnvironment::new(10).unwrap();
    let blockchain1 = Arc::new(RwLock::new(
        Blockchain::new(env1.clone(), NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let mempool1 = Mempool::new(Arc::clone(&blockchain1), MempoolConfig::default());

    let keypair =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer = BlockProducer::new(
        Arc::clone(&blockchain1),
        Arc::clone(&mempool1),
        keypair.clone(),
    );

    // The minimum number of macro blocks necessary so that we have one election block and one
    // checkpoint block to push.
    let num_macro_blocks = (policy::BATCHES_PER_EPOCH + 1) as usize;

    // Produce the blocks.
    produce_macro_blocks(num_macro_blocks, &producer, &blockchain1);

    let net1 = Arc::new(hub.new_network());
    let consensus1 = Consensus::from_network(
        env1,
        blockchain1,
        mempool1,
        Arc::clone(&net1),
        Box::pin(MockHistorySyncStream {
            network: Arc::clone(&net1),
        }),
    )
    .await;

    // Setup second peer (not synced yet).
    let env2 = VolatileEnvironment::new(10).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain2 = Arc::new(RwLock::new(
        Blockchain::new(env2.clone(), NetworkId::UnitAlbatross, time).unwrap(),
    ));
    let mempool2 = Mempool::new(Arc::clone(&blockchain2), MempoolConfig::default());

    let net2 = Arc::new(hub.new_network());
    let consensus2 = Consensus::from_network(
        env2,
        blockchain2,
        mempool2,
        Arc::clone(&net2),
        Box::pin(MockHistorySyncStream {
            network: Arc::clone(&net2),
        }),
    )
    .await;

    // Connect the two peers.
    let mut stream = consensus2.network.subscribe_events();
    net1.dial_mock(&net2);
    // Then wait for connection to be established.
    let _ = stream.next().await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await; // FIXME, Prof. Berrang told me to do this

    // Test ingredients:
    // Request hashes
    let agent = ConsensusAgent::new(Arc::clone(&net2.get_peers()[0]));
    let hashes = agent
        .request_block_hashes(
            vec![consensus2.blockchain.read().head_hash()],
            3,
            RequestBlockHashesFilter::ElectionAndLatestCheckpoint,
        )
        .await
        .expect("Should yield hashes")
        .hashes
        .expect("Should contain hashes");
    assert_eq!(hashes.len(), 2);
    assert_eq!(
        hashes[0].1,
        consensus1.blockchain.read().election_head_hash()
    );
    assert_eq!(hashes[1].1, consensus1.blockchain.read().macro_head_hash());

    // Request epoch
    let epoch = agent
        .request_epoch(consensus1.blockchain.read().election_head_hash())
        .await
        .expect("Should yield epoch");
    let block1 = epoch.block.expect("Should have block");

    assert_eq!(epoch.history_len, 3);
    assert_eq!(
        block1.hash(),
        consensus1.blockchain.read().election_head_hash()
    );

    let epoch = agent
        .request_epoch(consensus1.blockchain.read().macro_head_hash())
        .await
        .expect("Should yield epoch");
    let block2 = epoch.block.expect("Should have block");

    assert_eq!(epoch.history_len, 1);
    assert_eq!(
        block2.hash(),
        consensus1.blockchain.read().macro_head_hash()
    );

    // Request history chunk.
    let chunk = agent
        .request_history_chunk(1, block1.block_number(), 0)
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

    let chunk = agent
        .request_history_chunk(2, block2.block_number(), 0)
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
